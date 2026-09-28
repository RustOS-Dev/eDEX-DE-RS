//! `.socket2.sock` event stream parsing.

use std::{
    io::{BufRead, BufReader},
    os::unix::net::UnixStream,
    path::Path,
};

use anyhow::{Context, Result};

#[derive(Clone, Debug, PartialEq)]
pub enum HyprEvent {
    Workspace {
        id: Option<i32>,
        name: String,
    },
    FocusedMonitor {
        monitor: String,
        workspace: String,
    },
    ActiveWindow {
        class: String,
        title: String,
    },
    Fullscreen(bool),
    MonitorAdded(String),
    MonitorRemoved(String),
    CreateWorkspace {
        id: Option<i32>,
        name: String,
    },
    DestroyWorkspace {
        id: Option<i32>,
        name: String,
    },
    MoveWorkspace {
        name: String,
        monitor: String,
    },
    RenameWorkspace {
        id: i32,
        name: String,
    },
    ActiveLayout {
        keyboard: String,
        layout: String,
    },
    OpenWindow {
        address: String,
        workspace: String,
        class: String,
        title: String,
    },
    CloseWindow(String),
    MoveWindow {
        address: String,
        workspace: String,
    },
    OpenLayer(String),
    CloseLayer(String),
    Submap(String),
    Urgent(String),
    WindowTitle {
        address: String,
        title: Option<String>,
    },
    ConfigReloaded,
    Screencast {
        active: bool,
        owner: String,
    },
    Bell(Option<String>),
    Other {
        name: String,
        data: String,
    },
}

/// Parse one `EVENT>>DATA` line.
pub fn parse_line(line: &str) -> Option<HyprEvent> {
    let (name, data) = line.trim_end_matches(['\n', '\r']).split_once(">>")?;
    let fields = |n: usize| -> Vec<String> { data.splitn(n, ',').map(|s| s.to_string()).collect() };
    let ev = match name {
        "workspacev2" => {
            let f = fields(2);
            HyprEvent::Workspace {
                id: f.first().and_then(|s| s.parse().ok()),
                name: f.get(1).cloned().unwrap_or_default(),
            }
        }
        "workspace" => HyprEvent::Workspace {
            id: None,
            name: data.to_string(),
        },
        "focusedmon" => {
            let f = fields(2);
            HyprEvent::FocusedMonitor {
                monitor: f.first().cloned().unwrap_or_default(),
                workspace: f.get(1).cloned().unwrap_or_default(),
            }
        }
        "activewindow" => {
            let f = fields(2);
            HyprEvent::ActiveWindow {
                class: f.first().cloned().unwrap_or_default(),
                title: f.get(1).cloned().unwrap_or_default(),
            }
        }
        "fullscreen" => HyprEvent::Fullscreen(data == "1"),
        "monitoradded" => HyprEvent::MonitorAdded(data.to_string()),
        "monitorremoved" => HyprEvent::MonitorRemoved(data.to_string()),
        "createworkspacev2" => {
            let f = fields(2);
            HyprEvent::CreateWorkspace {
                id: f.first().and_then(|s| s.parse().ok()),
                name: f.get(1).cloned().unwrap_or_default(),
            }
        }
        "createworkspace" => HyprEvent::CreateWorkspace {
            id: None,
            name: data.to_string(),
        },
        "destroyworkspacev2" => {
            let f = fields(2);
            HyprEvent::DestroyWorkspace {
                id: f.first().and_then(|s| s.parse().ok()),
                name: f.get(1).cloned().unwrap_or_default(),
            }
        }
        "destroyworkspace" => HyprEvent::DestroyWorkspace {
            id: None,
            name: data.to_string(),
        },
        "moveworkspace" => {
            let f = fields(2);
            HyprEvent::MoveWorkspace {
                name: f.first().cloned().unwrap_or_default(),
                monitor: f.get(1).cloned().unwrap_or_default(),
            }
        }
        "renameworkspace" => {
            let f = fields(2);
            HyprEvent::RenameWorkspace {
                id: f.first().and_then(|s| s.parse().ok()).unwrap_or(0),
                name: f.get(1).cloned().unwrap_or_default(),
            }
        }
        "activelayout" => {
            let f = fields(2);
            HyprEvent::ActiveLayout {
                keyboard: f.first().cloned().unwrap_or_default(),
                layout: f.get(1).cloned().unwrap_or_default(),
            }
        }
        "openwindow" => {
            let f = fields(4);
            HyprEvent::OpenWindow {
                address: f.first().cloned().unwrap_or_default(),
                workspace: f.get(1).cloned().unwrap_or_default(),
                class: f.get(2).cloned().unwrap_or_default(),
                title: f.get(3).cloned().unwrap_or_default(),
            }
        }
        "closewindow" => HyprEvent::CloseWindow(data.to_string()),
        "movewindow" => {
            let f = fields(2);
            HyprEvent::MoveWindow {
                address: f.first().cloned().unwrap_or_default(),
                workspace: f.get(1).cloned().unwrap_or_default(),
            }
        }
        "openlayer" => HyprEvent::OpenLayer(data.to_string()),
        "closelayer" => HyprEvent::CloseLayer(data.to_string()),
        "submap" => HyprEvent::Submap(data.to_string()),
        "urgent" => HyprEvent::Urgent(data.to_string()),
        "windowtitlev2" => {
            let f = fields(2);
            HyprEvent::WindowTitle {
                address: f.first().cloned().unwrap_or_default(),
                title: f.get(1).cloned(),
            }
        }
        "windowtitle" => HyprEvent::WindowTitle {
            address: data.to_string(),
            title: None,
        },
        "configreloaded" => HyprEvent::ConfigReloaded,
        "screencast" => {
            let f = fields(2);
            HyprEvent::Screencast {
                active: f.first().map(|s| s == "1").unwrap_or(false),
                owner: f.get(1).cloned().unwrap_or_default(),
            }
        }
        "bell" => HyprEvent::Bell(if data.is_empty() {
            None
        } else {
            Some(data.to_string())
        }),
        // v2 variants we do not need separately are folded into their v1 handling above.
        "workspacev1" | "focusedmonv2" | "activewindowv2" | "monitoraddedv2"
        | "monitorremovedv2" | "moveworkspacev2" | "movewindowv2" | "activespecial"
        | "activespecialv2" | "changefloatingmode" | "pin" | "minimized" | "screencastv2"
        | "kill" | "togglegroup" | "moveintogroup" | "moveoutofgroup" | "ignoregrouplock"
        | "lockgroups" => {
            return None;
        }
        other => HyprEvent::Other {
            name: other.to_string(),
            data: data.to_string(),
        },
    };
    Some(ev)
}

/// Buffered reader over the event socket.
pub struct EventStream {
    reader: BufReader<UnixStream>,
}

impl EventStream {
    pub fn connect(instance_dir: &Path) -> Result<Self> {
        let path = instance_dir.join(".socket2.sock");
        let stream = UnixStream::connect(&path)
            .with_context(|| format!("connecting to {}", path.display()))?;
        stream.set_nonblocking(true)?;
        Ok(Self {
            reader: BufReader::new(stream),
        })
    }

    /// The underlying stream (for registering with calloop).
    pub fn stream(&self) -> &UnixStream {
        self.reader.get_ref()
    }

    /// Read every complete line currently available. Returns `Ok(false)` on EOF.
    pub fn drain(&mut self, out: &mut Vec<HyprEvent>) -> Result<bool> {
        loop {
            let mut line = String::new();
            match self.reader.read_line(&mut line) {
                Ok(0) => return Ok(false),
                Ok(_) => {
                    if let Some(ev) = parse_line(&line) {
                        out.push(ev);
                    }
                }
                Err(e) if e.kind() == std::io::ErrorKind::WouldBlock => return Ok(true),
                Err(e) if e.kind() == std::io::ErrorKind::Interrupted => continue,
                Err(e) => return Err(e.into()),
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_common_events() {
        assert_eq!(
            parse_line("workspacev2>>3,three\n"),
            Some(HyprEvent::Workspace {
                id: Some(3),
                name: "three".into()
            })
        );
        assert_eq!(
            parse_line("activewindow>>kitty,~ — fish"),
            Some(HyprEvent::ActiveWindow {
                class: "kitty".into(),
                title: "~ — fish".into()
            })
        );
        assert_eq!(
            parse_line("openwindow>>5f0a,2,firefox,Mozilla, Firefox"),
            Some(HyprEvent::OpenWindow {
                address: "5f0a".into(),
                workspace: "2".into(),
                class: "firefox".into(),
                title: "Mozilla, Firefox".into()
            })
        );
        assert_eq!(
            parse_line("configreloaded>>"),
            Some(HyprEvent::ConfigReloaded)
        );
        assert_eq!(
            parse_line("activelayout>>at-translated-set-2-keyboard,English (US)"),
            Some(HyprEvent::ActiveLayout {
                keyboard: "at-translated-set-2-keyboard".into(),
                layout: "English (US)".into()
            })
        );
        assert_eq!(parse_line("garbage"), None);
        assert_eq!(parse_line("activewindowv2>>5f0a"), None);
    }
}
