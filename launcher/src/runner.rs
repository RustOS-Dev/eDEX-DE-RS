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
    pub force_terminal: bool,
}

impl Default for LaunchOptions {
    fn default() -> Self {
        Self {
            terminal_command: "kitty -e".into(),
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

/// Launch an entry directly (the shell launches through the window manager first when it can).
pub fn launch(app: &AppEntry, opts: &LaunchOptions) -> Result<()> {
    let cmd = command_line(app, opts);
    info!(app = %app.id, %cmd, "launching");
    spawn_detached(&cmd)
}

/// Spawn a shell command detached from the shell. Never blocks and never leaves a zombie: `sh`
/// runs in a new session, starts the command in the background and exits at once, so the
/// command is reparented to init (and survives a shell restart).
pub fn spawn_detached(cmd: &str) -> Result<()> {
    // In its own transient scope when systemd is around: otherwise the app lives in the shell's
    // service cgroup and a shell restart (or crash + Restart=on-failure) would kill it.
    let line = if in_systemd_user_session() {
        format!(
            "systemd-run --user --scope --quiet --collect -- sh -c {} &",
            crate::desktop::shell_quote(cmd)
        )
    } else {
        format!("{{ {cmd}\n}} &")
    };
    let mut child = Command::new("sh");
    child
        .arg("-c")
        .arg(line)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null());
    unsafe {
        child.pre_exec(|| {
            if libc::setsid() < 0 {
                return Err(std::io::Error::last_os_error());
            }
            Ok(())
        });
    }
    let mut child = child.spawn().with_context(|| format!("spawning `{cmd}`"))?;
    // `sh` exits as soon as the command is in the background; reap it.
    std::thread::spawn(move || {
        let _ = child.wait();
    });
    Ok(())
}

fn in_systemd_user_session() -> bool {
    let bus = std::env::var_os("XDG_RUNTIME_DIR")
        .map(|d| PathBuf::from(d).join("systemd/private"))
        .is_some_and(|p| p.exists());
    bus && std::path::Path::new("/usr/bin/systemd-run").exists()
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
            "cd '/tmp' && kitty -e htop"
        );
    }

    #[test]
    fn detached_spawn_returns_immediately() {
        let start = std::time::Instant::now();
        spawn_detached("sleep 2").unwrap();
        assert!(start.elapsed().as_millis() < 1000);
    }
}
