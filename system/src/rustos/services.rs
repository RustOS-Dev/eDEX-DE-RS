//! Services on RustOS: there is no service manager; `/etc/rc` (and `/storage/etc/rc.local`)
//! start daemons at boot. Each command line there is a "unit": running when a process with
//! that program's name exists; start runs the line again, stop signals the processes.

use anyhow::{anyhow, Result};

use crate::{
    services::{ServicesState, UnitAction, UnitInfo},
    CommandRunner,
};

/// One command started at boot.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RcCommand {
    /// Program name (the unit's name).
    pub name: String,
    /// The line as written, without a trailing `&`.
    pub command: String,
    pub file: &'static str,
}

/// Commands of an rc script: every non-comment line whose first word is a program (not a
/// shell keyword or a plain `mkdir`).
pub fn parse_rc(text: &str, file: &'static str) -> Vec<RcCommand> {
    const SKIP: [&str; 20] = [
        "if", "then", "else", "elif", "fi", "for", "do", "done", "while", "case", "esac", "export",
        "mkdir", "echo", "cd", "test", "[", "true", "false", "exit",
    ];
    let mut out: Vec<RcCommand> = Vec::new();
    for line in text.lines() {
        let l = line.split('#').next().unwrap_or("").trim();
        if l.is_empty() || l.contains('=') && !l.contains(' ') {
            continue;
        }
        let cmd = l.trim_end_matches('&').trim();
        let Some(first) = cmd.split_whitespace().next() else {
            continue;
        };
        let name = first.rsplit('/').next().unwrap_or(first);
        if SKIP.contains(&name) || name.contains('=') {
            continue;
        }
        if out.iter().any(|c| c.name == name) {
            continue;
        }
        out.push(RcCommand {
            name: name.to_string(),
            command: cmd.to_string(),
            file,
        });
    }
    out
}

fn rc_commands() -> Vec<RcCommand> {
    let mut v = parse_rc(
        &std::fs::read_to_string("/etc/rc").unwrap_or_default(),
        "/etc/rc",
    );
    for c in parse_rc(
        &std::fs::read_to_string("/storage/etc/rc.local").unwrap_or_default(),
        "/storage/etc/rc.local",
    ) {
        if !v.iter().any(|x| x.name == c.name) {
            v.push(c);
        }
    }
    v
}

/// (pid, comm) of every process.
fn processes() -> Vec<(i32, String)> {
    let Ok(dir) = std::fs::read_dir("/proc") else {
        return Vec::new();
    };
    dir.flatten()
        .filter_map(|e| {
            let pid: i32 = e.file_name().to_str()?.parse().ok()?;
            let comm = std::fs::read_to_string(e.path().join("comm")).ok()?;
            Some((pid, comm.trim().to_string()))
        })
        .collect()
}

pub fn query(_r: &dyn CommandRunner, user: bool) -> ServicesState {
    if user {
        return ServicesState {
            user,
            units: Vec::new(),
        };
    }
    let procs = processes();
    let units = rc_commands()
        .into_iter()
        .map(|c| {
            let running = procs.iter().any(|(_, n)| *n == c.name);
            UnitInfo {
                description: format!("{} ({})", c.command, c.file),
                active: if running { "active" } else { "inactive" }.into(),
                sub: if running { "running" } else { "exited" }.into(),
                enabled: "enabled".into(),
                name: c.name,
            }
        })
        .collect();
    ServicesState { user, units }
}

fn stop(name: &str) -> Result<()> {
    let me = std::process::id() as i32;
    let pids: Vec<i32> = processes()
        .into_iter()
        .filter(|(p, n)| n == name && *p != me && *p != 1)
        .map(|(p, _)| p)
        .collect();
    if pids.is_empty() {
        return Err(anyhow!("{name} is not running"));
    }
    for p in pids {
        // SAFETY: plain kill(2).
        unsafe { libc::kill(p, libc::SIGTERM) };
    }
    Ok(())
}

pub fn act(_r: &dyn CommandRunner, user: bool, unit: &str, action: UnitAction) -> Result<()> {
    if user {
        return Err(anyhow!("RustOS has no user services"));
    }
    let cmd = rc_commands()
        .into_iter()
        .find(|c| c.name == unit)
        .ok_or_else(|| anyhow!("{unit} is not started by /etc/rc"))?;
    match action {
        UnitAction::Start => launcher::runner::spawn_detached(&cmd.command),
        UnitAction::Stop => stop(unit),
        UnitAction::Restart => {
            let _ = stop(unit);
            std::thread::sleep(std::time::Duration::from_millis(500));
            launcher::runner::spawn_detached(&cmd.command)
        }
        _ => Err(anyhow!(
            "services start from /etc/rc; edit it to change what starts"
        )),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_etc_rc() {
        let rc = "# System startup script (run by init).\nmkdir -p /tmp /var/log /mnt\nifup -a -q\nwifi auto -q &\nexport FOO=1\nedex-session &   # the desktop\n/usr/local/bin/seatd -g video &\n";
        let c = parse_rc(rc, "/etc/rc");
        let names: Vec<_> = c.iter().map(|c| c.name.as_str()).collect();
        assert_eq!(names, ["ifup", "wifi", "edex-session", "seatd"]);
        assert_eq!(c[1].command, "wifi auto -q");
        assert_eq!(c[3].command, "/usr/local/bin/seatd -g video");
    }
}
