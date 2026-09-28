//! Blocking client used by `edex-de ipc …` and by scripts.

use std::{
    io::{BufRead, BufReader, Write},
    os::unix::net::UnixStream,
    path::Path,
    time::Duration,
};

use anyhow::{Context, Result};

use crate::proto::{Request, Response};

pub fn send(path: &Path, request: &Request) -> Result<Response> {
    let mut stream = UnixStream::connect(path).with_context(|| format!("edex-de is not running (no socket at {})", path.display()))?;
    stream.set_read_timeout(Some(Duration::from_secs(5)))?;
    let mut line = serde_json::to_string(request)?;
    line.push('\n');
    stream.write_all(line.as_bytes())?;
    stream.flush()?;
    let mut reader = BufReader::new(stream);
    let mut reply = String::new();
    reader.read_line(&mut reply)?;
    serde_json::from_str(reply.trim()).context("malformed reply")
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::server::IpcServer;

    #[test]
    fn roundtrip_over_unix_socket() {
        let dir = std::env::temp_dir().join(format!("edex-ipc-test-{}", std::process::id()));
        let path = dir.join("ipc.sock");
        let server = IpcServer::bind(path.clone()).unwrap();
        let p2 = path.clone();
        let client = std::thread::spawn(move || send(&p2, &Request::Ping).unwrap());
        let mut got = None;
        for _ in 0..200 {
            let pending = server.accept_all();
            if let Some(p) = pending.into_iter().next() {
                assert_eq!(p.request, Request::Ping);
                p.reply(&Response::with_data(serde_json::json!({"pong": true})));
                got = Some(());
                break;
            }
            std::thread::sleep(Duration::from_millis(10));
        }
        assert!(got.is_some());
        let resp = client.join().unwrap();
        assert!(resp.ok);
        assert_eq!(resp.data.unwrap()["pong"], true);
        drop(server);
        assert!(!path.exists());
        let _ = std::fs::remove_dir_all(dir);
    }
}
