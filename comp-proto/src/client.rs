//! Blocking request client and a non-blocking event reader.

use std::{
    io::{BufRead, BufReader, ErrorKind, Read, Write},
    os::{fd::AsRawFd, unix::net::UnixStream},
    path::Path,
    time::Duration,
};

use anyhow::{anyhow, bail, Context, Result};

use crate::proto::{Event, Reply, Request};

/// Send one request to the compositor at [`crate::socket_path`] and return its reply data
/// (`Null` when the reply has none).
pub fn call(request: &Request) -> Result<serde_json::Value> {
    let path = crate::socket_path().ok_or_else(|| anyhow!("edex-comp is not running"))?;
    call_at(&path, request)
}

pub fn call_at(path: &Path, request: &Request) -> Result<serde_json::Value> {
    let mut stream = UnixStream::connect(path)
        .with_context(|| format!("edex-comp is not running (no socket at {})", path.display()))?;
    stream.set_read_timeout(Some(Duration::from_secs(5)))?;
    write_line(&mut stream, request)?;
    let mut reader = BufReader::new(stream);
    let reply = read_reply(&mut reader)?;
    if reply.ok {
        Ok(reply.data.unwrap_or(serde_json::Value::Null))
    } else {
        bail!(reply.error.unwrap_or_else(|| "request failed".into()))
    }
}

fn write_line(stream: &mut UnixStream, request: &Request) -> Result<()> {
    let mut line = serde_json::to_string(request)?;
    line.push('\n');
    stream.write_all(line.as_bytes())?;
    stream.flush()?;
    Ok(())
}

fn read_reply(reader: &mut impl BufRead) -> Result<Reply> {
    let mut line = String::new();
    if reader.read_line(&mut line)? == 0 {
        bail!("edex-comp closed the connection");
    }
    serde_json::from_str(line.trim()).context("malformed reply from edex-comp")
}

/// A subscribed connection. Register [`EventReader::fd`] with the event loop (level-triggered)
/// and call [`EventReader::drain`] when it is readable.
pub struct EventReader {
    stream: UnixStream,
    buf: Vec<u8>,
}

impl EventReader {
    pub fn connect(path: &Path) -> Result<Self> {
        let mut stream = UnixStream::connect(path)
            .with_context(|| format!("connecting to {}", path.display()))?;
        stream.set_read_timeout(Some(Duration::from_secs(5)))?;
        write_line(&mut stream, &Request::Subscribe)?;
        // Read the reply byte by byte so no event that follows it is swallowed by a buffer.
        let mut line = Vec::new();
        let mut byte = [0u8; 1];
        loop {
            match stream.read(&mut byte)? {
                0 => bail!("edex-comp closed the event stream"),
                _ if byte[0] == b'\n' => break,
                _ => line.push(byte[0]),
            }
        }
        let reply: Reply = serde_json::from_slice(&line).context("malformed subscribe reply")?;
        if !reply.ok {
            bail!(reply.error.unwrap_or_else(|| "subscribe refused".into()));
        }
        stream.set_nonblocking(true)?;
        Ok(Self {
            stream,
            buf: Vec::new(),
        })
    }

    pub fn fd(&self) -> std::os::fd::RawFd {
        self.stream.as_raw_fd()
    }

    pub fn stream(&self) -> &UnixStream {
        &self.stream
    }

    /// Read every complete event available now. `Err` means the compositor went away.
    pub fn drain(&mut self) -> Result<Vec<Event>> {
        let mut chunk = [0u8; 8192];
        loop {
            match self.stream.read(&mut chunk) {
                Ok(0) => bail!("edex-comp closed the event stream"),
                Ok(n) => self.buf.extend_from_slice(&chunk[..n]),
                Err(e) if e.kind() == ErrorKind::WouldBlock => break,
                Err(e) if e.kind() == ErrorKind::Interrupted => continue,
                Err(e) => return Err(e.into()),
            }
        }
        let mut events = Vec::new();
        while let Some(pos) = self.buf.iter().position(|&b| b == b'\n') {
            let line: Vec<u8> = self.buf.drain(..=pos).collect();
            match serde_json::from_slice::<Event>(&line[..line.len() - 1]) {
                Ok(ev) => events.push(ev),
                // A newer compositor may send events this client does not know.
                Err(_) => continue,
            }
        }
        Ok(events)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::os::unix::net::UnixListener;

    #[test]
    fn call_and_subscribe_over_a_socket() {
        let dir = std::env::temp_dir().join(format!("comp-proto-test-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("comp.sock");
        let _ = std::fs::remove_file(&path);
        let listener = UnixListener::bind(&path).unwrap();
        let server = std::thread::spawn(move || {
            for _ in 0..2 {
                let (stream, _) = listener.accept().unwrap();
                let mut reader = BufReader::new(stream.try_clone().unwrap());
                let mut line = String::new();
                reader.read_line(&mut line).unwrap();
                let mut out = stream;
                match serde_json::from_str::<Request>(line.trim()).unwrap() {
                    Request::Version => {
                        out.write_all(b"{\"ok\":true,\"data\":{\"version\":\"1\"}}\n")
                            .unwrap();
                    }
                    Request::Subscribe => {
                        // Reply and first event in one write: the reader must keep the event.
                        out.write_all(
                            b"{\"ok\":true}\n{\"event\":\"bell\"}\n{\"event\":\"focus\",",
                        )
                        .unwrap();
                        std::thread::sleep(Duration::from_millis(50));
                        out.write_all(b"\"id\":4}\n{\"event\":\"from-the-future\"}\n")
                            .unwrap();
                        std::thread::sleep(Duration::from_millis(100));
                    }
                    other => panic!("unexpected {other:?}"),
                }
            }
        });
        let v = call_at(&path, &Request::Version).unwrap();
        assert_eq!(v["version"], "1");
        let mut reader = EventReader::connect(&path).unwrap();
        let mut got = Vec::new();
        for _ in 0..50 {
            got.extend(reader.drain().unwrap());
            if got.len() >= 2 {
                break;
            }
            std::thread::sleep(Duration::from_millis(10));
        }
        assert_eq!(got, vec![Event::Bell, Event::Focus { id: Some(4) }]);
        server.join().unwrap();
        let _ = std::fs::remove_dir_all(dir);
    }
}
