//! Non-blocking IPC server; the event loop polls the listener and each connection.

use std::{
    io::{BufRead, BufReader, ErrorKind, Write},
    os::unix::{
        fs::PermissionsExt,
        net::{UnixListener, UnixStream},
    },
    path::PathBuf,
};

use anyhow::{Context, Result};
use tracing::{debug, warn};

use crate::proto::{Request, Response};

/// A parsed request waiting for a reply.
pub struct Pending {
    pub request: Request,
    stream: UnixStream,
}

impl Pending {
    pub fn reply(mut self, response: &Response) {
        let mut line = serde_json::to_string(response).unwrap_or_else(|_| "{\"ok\":false}".into());
        line.push('\n');
        let _ = self.stream.set_nonblocking(false);
        let _ = self.stream.write_all(line.as_bytes());
        let _ = self.stream.flush();
    }
}

pub struct IpcServer {
    listener: UnixListener,
    path: PathBuf,
}

impl IpcServer {
    /// Bind the socket, replacing a stale one left by a crashed instance.
    pub fn bind(path: PathBuf) -> Result<Self> {
        if let Some(dir) = path.parent() {
            std::fs::create_dir_all(dir).with_context(|| format!("creating {}", dir.display()))?;
            let _ = std::fs::set_permissions(dir, std::fs::Permissions::from_mode(0o700));
        }
        if path.exists() {
            match UnixStream::connect(&path) {
                Ok(_) => anyhow::bail!(
                    "another edex-de instance is already listening on {}",
                    path.display()
                ),
                Err(_) => {
                    std::fs::remove_file(&path).ok();
                }
            }
        }
        let listener =
            UnixListener::bind(&path).with_context(|| format!("binding {}", path.display()))?;
        listener.set_nonblocking(true)?;
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o600))?;
        debug!(path = %path.display(), "ipc listening");
        Ok(Self { listener, path })
    }

    pub fn listener(&self) -> &UnixListener {
        &self.listener
    }

    /// Accept every waiting connection and read its request line.
    pub fn accept_all(&self) -> Vec<Pending> {
        let mut out = Vec::new();
        loop {
            match self.listener.accept() {
                Ok((stream, _)) => {
                    if let Some(p) = read_request(stream) {
                        out.push(p);
                    }
                }
                Err(e) if e.kind() == ErrorKind::WouldBlock => break,
                Err(e) => {
                    warn!("ipc accept failed: {e}");
                    break;
                }
            }
        }
        out
    }
}

fn read_request(stream: UnixStream) -> Option<Pending> {
    let _ = stream.set_nonblocking(false);
    let _ = stream.set_read_timeout(Some(std::time::Duration::from_millis(500)));
    let mut reader = BufReader::new(stream.try_clone().ok()?);
    let mut line = String::new();
    if reader.read_line(&mut line).ok()? == 0 {
        return None;
    }
    match serde_json::from_str::<Request>(line.trim()) {
        Ok(request) => Some(Pending { request, stream }),
        Err(e) => {
            let mut s = stream;
            let _ = s.write_all(
                format!(
                    "{}\n",
                    serde_json::to_string(&Response::err(format!("bad request: {e}"))).unwrap()
                )
                .as_bytes(),
            );
            None
        }
    }
}

impl Drop for IpcServer {
    fn drop(&mut self) {
        let _ = std::fs::remove_file(&self.path);
    }
}
