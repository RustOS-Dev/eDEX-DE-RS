//! Control protocol of `edex-comp`, the eDEX compositor.
//!
//! Newline-delimited JSON over the Unix socket [`socket_path`]. A connection sends one
//! [`Request`] per line and reads one [`Reply`] per line. A connection that sends
//! [`Request::Subscribe`] gets a `{"ok":true}` reply and then a stream of [`Event`] lines until it
//! closes; it sends nothing more.

pub mod client;
pub mod proto;

pub use client::{call, call_at, EventReader};
pub use proto::*;

use std::path::PathBuf;

/// Environment variable naming the control socket; edex-comp exports it to every client it
/// starts.
pub const SOCKET_ENV: &str = "EDEX_COMP_SOCKET";

/// Path of the control socket: `$EDEX_COMP_SOCKET`, else `$XDG_RUNTIME_DIR/edex-comp.sock`.
pub fn socket_path() -> Option<PathBuf> {
    if let Some(p) = std::env::var_os(SOCKET_ENV) {
        return Some(PathBuf::from(p));
    }
    let runtime = std::env::var_os("XDG_RUNTIME_DIR")?;
    Some(PathBuf::from(runtime).join("edex-comp.sock"))
}
