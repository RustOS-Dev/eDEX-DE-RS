//! Launch applications detached from the shell process.

use std::{
    os::unix::process::CommandExt,
    path::PathBuf,
    process::{Command, Stdio},
};

use anyhow::{Context, Result};
use tracing::info;

use crate::desktop::{expand_exec, AppEntry};

#[derive(Clone, Debug)]
pub struct LaunchOptions {
    /// Command prefix used for `Terminal=true` entries or forced terminal launches.
    pub terminal_command: String,
    /// When true, launch through edex-comp so it gets the session environment and opens on the
    /// current workspace.
    pub via_compositor: bool,
    pub force_terminal: bool,
}

impl Default for LaunchOptions {
    fn default() -> Self {
        Self {
            terminal_command: "foot".into(),
            via_compositor: false,
            force_terminal: false,
        }
    }
}

/// Build the shell command line for an entry.
pub fn command_line(app: &AppEntry, opts: &LaunchOptions) -> String {
    let exec = expand_exec(app);
    let mut cmd = if app.terminal || opts.force_terminal {
        format!("{} {}", opts.terminal_command, exec)
    } else {
        exec
    };
    if let Some(path) = &app.path {
        cmd = format!("cd {} && {}", crate::desktop::shell_quote(path), cmd);
    }
    cmd
}

/// Launch an entry. Never blocks and never leaves a zombie: the child is double-forked via
/// `setsid` so it is reparented to init.
pub fn launch(app: &AppEntry, opts: &LaunchOptions) -> Result<()> {
    let cmd = command_line(app, opts);
    info!(app = %app.id, %cmd, "launching");
    spawn_detached(&cmd, opts.via_compositor)
}

/// Spawn an arbitrary shell command detached from the shell.
pub fn spawn_detached(cmd: &str, via_compositor: bool) -> Result<()> {
    if via_compositor {
        if let Some(socket) = comp::CompSocket::from_env() {
            match socket.exec(cmd) {
                Ok(()) => return Ok(()),
                Err(e) => {
                    tracing::warn!("launching through edex-comp failed, spawning directly: {e:#}")
                }
            }
        }
    }
    let mut command = Command::new("/bin/sh");
    command
        .arg("-c")
        .arg(cmd)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null());
    // In its own session, so a shell restart (or crash) does not take the app with it.
    // SAFETY: setsid is async-signal-safe.
    unsafe {
        command.pre_exec(|| {
            libc::setsid();
            Ok(())
        });
    }
    let mut child = command
        .spawn()
        .with_context(|| format!("spawning `{cmd}`"))?;
    std::thread::spawn(move || {
        let _ = child.wait();
    });
    Ok(())
}

/// Path of the launch history file.
pub fn history_path() -> PathBuf {
    let state = std::env::var("XDG_STATE_HOME")
        .map(PathBuf::from)
        .unwrap_or_else(|_| {
            PathBuf::from(std::env::var("HOME").unwrap_or_else(|_| "/tmp".into()))
                .join(".local/state")
        });
    state.join("edex-de").join("launcher-history.json")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn terminal_entries_get_wrapped() {
        let app = AppEntry {
            id: "htop".into(),
            name: "htop".into(),
            generic_name: None,
            exec: "htop %F".into(),
            icon: None,
            categories: vec![],
            keywords: vec![],
            comment: None,
            terminal: true,
            path: Some("/tmp".into()),
            startup_wm_class: None,
            file: PathBuf::from("/usr/share/applications/htop.desktop"),
        };
        assert_eq!(
            command_line(&app, &LaunchOptions::default()),
            "cd '/tmp' && foot htop"
        );
    }

    #[test]
    fn detached_spawn_returns_immediately() {
        let start = std::time::Instant::now();
        spawn_detached("sleep 2", false).unwrap();
        assert!(start.elapsed().as_millis() < 1000);
    }
}
