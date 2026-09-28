//! Shell IPC: newline-delimited JSON over a Unix socket in `$XDG_RUNTIME_DIR/edex-de/`.

pub mod client;
pub mod proto;
pub mod server;

pub use client::send;
pub use proto::{Request, Response, Target};
pub use server::{IpcServer, Pending};

use std::path::PathBuf;

/// Path of the IPC socket for this user.
pub fn socket_path() -> PathBuf {
    let base = std::env::var("XDG_RUNTIME_DIR")
        .map(PathBuf::from)
        .unwrap_or_else(|_| std::env::temp_dir().join(format!("edex-de-{}", whoami_uid())));
    base.join("edex-de").join("ipc.sock")
}

fn whoami_uid() -> u32 {
    // SAFETY: getuid has no preconditions.
    unsafe { libc_getuid() }
}

extern "C" {
    #[link_name = "getuid"]
    fn libc_getuid() -> u32;
}
