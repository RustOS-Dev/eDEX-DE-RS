//! The control socket (`comp_proto`): accepting requests and broadcasting events.

use std::{
    io::{BufRead, BufReader, ErrorKind, Write},
    os::unix::{
        fs::PermissionsExt,
        net::{UnixListener, UnixStream},
    },
    path::{Path, PathBuf},
    time::Duration,
};

use anyhow::{Context, Result};
use comp_proto::{Event, Reply, Request, Snapshot};
use tracing::{debug, warn};

pub struct ControlServer {
    listener: UnixListener,
    path: PathBuf,
    subscribers: Vec<UnixStream>,
}

/// A request read from a connection; reply through it.
pub struct Incoming {
    pub request: Request,
    stream: UnixStream,
}

impl Incoming {
    pub fn reply(mut self, reply: &Reply) {
        let mut line = serde_json::to_string(reply).unwrap_or_else(|_| "{\"ok\":false}".into());
        line.push('\n');
        let _ = self.stream.write_all(line.as_bytes());
    }

    /// Keep the connection as an event subscriber.
    pub fn into_stream(self) -> UnixStream {
        self.stream
    }
}

impl ControlServer {
    pub fn bind(path: &Path) -> Result<Self> {
        if let Some(dir) = path.parent() {
            std::fs::create_dir_all(dir).with_context(|| format!("creating {}", dir.display()))?;
        }
        if path.exists() {
            if UnixStream::connect(path).is_ok() {
                anyhow::bail!("another edex-comp is listening on {}", path.display());
            }
            let _ = std::fs::remove_file(path);
        }
        let listener =
            UnixListener::bind(path).with_context(|| format!("binding {}", path.display()))?;
        listener.set_nonblocking(true)?;
        std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o600))?;
        Ok(Self {
            listener,
            path: path.to_path_buf(),
            subscribers: Vec::new(),
        })
    }

    pub fn path(&self) -> &Path {
        &self.path
    }

    pub fn listener(&self) -> &UnixListener {
        &self.listener
    }

    /// Accept every waiting connection and read its request.
    pub fn accept(&self) -> Vec<Incoming> {
        let mut out = Vec::new();
        loop {
            match self.listener.accept() {
                Ok((stream, _)) => {
                    if let Some(i) = read_request(stream) {
                        out.push(i);
                    }
                }
                Err(e) if e.kind() == ErrorKind::WouldBlock => break,
                Err(e) => {
                    warn!("control socket accept failed: {e}");
                    break;
                }
            }
        }
        out
    }

    pub fn subscribe(&mut self, incoming: Incoming) {
        incoming.reply_ok_keep(&mut self.subscribers);
    }

    pub fn has_subscribers(&self) -> bool {
        !self.subscribers.is_empty()
    }

    /// Send events to every subscriber; drop the ones that cannot keep up or went away.
    pub fn broadcast(&mut self, events: &[Event]) {
        if events.is_empty() || self.subscribers.is_empty() {
            return;
        }
        let mut payload = String::new();
        for e in events {
            if let Ok(line) = serde_json::to_string(e) {
                payload.push_str(&line);
                payload.push('\n');
            }
        }
        self.subscribers
            .retain_mut(|s| match s.write_all(payload.as_bytes()) {
                Ok(()) => true,
                Err(e) => {
                    debug!("dropping event subscriber: {e}");
                    false
                }
            });
    }
}

impl Incoming {
    fn reply_ok_keep(self, subscribers: &mut Vec<UnixStream>) {
        let mut stream = self.stream;
        if stream.write_all(b"{\"ok\":true}\n").is_ok() && stream.set_nonblocking(true).is_ok() {
            subscribers.push(stream);
        }
    }
}

impl Drop for ControlServer {
    fn drop(&mut self) {
        let _ = std::fs::remove_file(&self.path);
    }
}

fn read_request(stream: UnixStream) -> Option<Incoming> {
    let _ = stream.set_nonblocking(false);
    let _ = stream.set_read_timeout(Some(Duration::from_millis(250)));
    let _ = stream.set_write_timeout(Some(Duration::from_millis(250)));
    let mut reader = BufReader::new(stream.try_clone().ok()?);
    let mut line = String::new();
    if reader.read_line(&mut line).ok()? == 0 {
        return None;
    }
    match serde_json::from_str::<Request>(line.trim()) {
        Ok(request) => Some(Incoming { request, stream }),
        Err(e) => {
            let mut s = stream;
            let reply = Reply::err(format!("bad request: {e}"));
            let _ = s.write_all(format!("{}\n", serde_json::to_string(&reply).ok()?).as_bytes());
            None
        }
    }
}

/// Events that turn `old` into `new`.
pub fn diff(old: &Snapshot, new: &Snapshot) -> Vec<Event> {
    let mut events = Vec::new();
    for w in &new.windows {
        match old.windows.iter().find(|o| o.id == w.id) {
            None => events.push(Event::WindowOpened { window: w.clone() }),
            Some(o) if o != w => events.push(Event::WindowChanged { window: w.clone() }),
            _ => {}
        }
    }
    for o in &old.windows {
        if !new.windows.iter().any(|w| w.id == o.id) {
            events.push(Event::WindowClosed { id: o.id });
        }
    }
    for m in &new.monitors {
        let before = old.monitors.iter().find(|o| o.name == m.name);
        if before.is_none_or(|o| o.active_workspace != m.active_workspace) {
            events.push(Event::Workspace {
                monitor: m.name.clone(),
                workspace: m.active_workspace,
            });
        }
    }
    if old.monitors != new.monitors {
        events.push(Event::Monitors {
            monitors: new.monitors.clone(),
        });
    }
    if old.workspaces != new.workspaces {
        events.push(Event::Workspaces {
            workspaces: new.workspaces.clone(),
        });
    }
    if old.focused != new.focused {
        events.push(Event::Focus { id: new.focused });
    }
    if old.keyboard_layout != new.keyboard_layout {
        events.push(Event::KeyboardLayout {
            name: new.keyboard_layout.clone(),
        });
    }
    if old.locked != new.locked {
        events.push(Event::Locked { locked: new.locked });
    }
    events
}

#[cfg(test)]
mod tests {
    use super::*;
    use comp_proto::{Monitor, Window, WindowMode};

    fn win(id: u64, title: &str) -> Window {
        Window {
            id,
            app_id: "foot".into(),
            title: title.into(),
            workspace: 1,
            mode: WindowMode::Tiled,
            ..Default::default()
        }
    }

    #[test]
    fn diffs_snapshots_into_events() {
        let mut a = Snapshot {
            windows: vec![win(1, "a"), win(2, "b")],
            monitors: vec![Monitor {
                name: "DP-1".into(),
                active_workspace: 1,
                ..Default::default()
            }],
            focused: Some(1),
            ..Default::default()
        };
        assert!(diff(&a, &a).is_empty());
        let mut b = a.clone();
        b.windows[0].title = "renamed".into();
        b.windows.remove(1);
        b.windows.push(win(3, "c"));
        b.monitors[0].active_workspace = 2;
        b.focused = Some(3);
        b.locked = true;
        let ev = diff(&a, &b);
        assert!(ev.contains(&Event::WindowChanged {
            window: b.windows[0].clone()
        }));
        assert!(ev.contains(&Event::WindowClosed { id: 2 }));
        assert!(ev.contains(&Event::WindowOpened {
            window: win(3, "c")
        }));
        assert!(ev.contains(&Event::Workspace {
            monitor: "DP-1".into(),
            workspace: 2
        }));
        assert!(ev.contains(&Event::Focus { id: Some(3) }));
        assert!(ev.contains(&Event::Locked { locked: true }));
        a.keyboard_layout = "German".into();
        assert!(diff(&b, &a).contains(&Event::KeyboardLayout {
            name: "German".into()
        }));
    }

    #[test]
    fn serves_requests_and_subscribers() {
        let dir = std::env::temp_dir().join(format!("edex-comp-ipc-{}", std::process::id()));
        let path = dir.join("comp.sock");
        let mut server = ControlServer::bind(&path).unwrap();
        let p = path.clone();
        let client = std::thread::spawn(move || {
            let v = comp_proto::call_at(&p, &Request::Version).unwrap();
            let mut events = comp_proto::EventReader::connect(&p).unwrap();
            let mut got = Vec::new();
            for _ in 0..200 {
                got.extend(events.drain().unwrap());
                if !got.is_empty() {
                    break;
                }
                std::thread::sleep(Duration::from_millis(10));
            }
            (v, got)
        });
        let mut served = 0;
        for _ in 0..400 {
            for incoming in server.accept() {
                match incoming.request {
                    Request::Version => {
                        incoming.reply(&Reply::with_data(serde_json::json!({"version": "t"})));
                    }
                    Request::Subscribe => server.subscribe(incoming),
                    _ => incoming.reply(&Reply::err("unexpected")),
                }
                served += 1;
            }
            if served == 2 {
                server.broadcast(&[Event::Bell]);
                break;
            }
            std::thread::sleep(Duration::from_millis(5));
        }
        let (v, got) = client.join().unwrap();
        assert_eq!(v["version"], "t");
        assert_eq!(got, vec![Event::Bell]);
        drop(server);
        assert!(!path.exists());
        let _ = std::fs::remove_dir_all(dir);
    }
}
