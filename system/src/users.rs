//! Local user accounts (accountsservice with passwd fallback).

use anyhow::Result;

use crate::runner::CommandRunner;

#[derive(Clone, Debug, Default, PartialEq)]
pub struct UserInfo {
    pub name: String,
    pub real_name: String,
    pub uid: u32,
    pub admin: bool,
    pub locked: bool,
    pub shell: String,
    pub home: String,
}

pub fn query(r: &dyn CommandRunner) -> Vec<UserInfo> {
    let passwd = std::fs::read_to_string("/etc/passwd").unwrap_or_default();
    let wheel: Vec<String> = std::fs::read_to_string("/etc/group")
        .ok()
        .and_then(|g| {
            g.lines().find(|l| l.starts_with("wheel:")).map(|l| {
                l.rsplit(':')
                    .next()
                    .unwrap_or("")
                    .split(',')
                    .map(|s| s.to_string())
                    .collect()
            })
        })
        .unwrap_or_default();
    let mut users: Vec<UserInfo> = passwd
        .lines()
        .filter_map(|l| {
            let f: Vec<&str> = l.split(':').collect();
            if f.len() < 7 {
                return None;
            }
            let uid: u32 = f[2].parse().ok()?;
            if !(1000..60000).contains(&uid) || f[6].ends_with("nologin") || f[6].ends_with("false")
            {
                return None;
            }
            Some(UserInfo {
                name: f[0].into(),
                real_name: f[4].split(',').next().unwrap_or("").into(),
                uid,
                admin: wheel.iter().any(|w| w == f[0]),
                locked: false,
                shell: f[6].into(),
                home: f[5].into(),
            })
        })
        .collect();
    for u in &mut users {
        if let Ok(out) = r.run("passwd", &["-S", &u.name]) {
            if out.ok() {
                // "name L 2024-01-01 0 99999 7 -1" → L = locked, P = usable
                u.locked = out.stdout.split_whitespace().nth(1) == Some("L");
            }
        }
    }
    users
}

pub fn set_real_name(r: &dyn CommandRunner, user: &str, name: &str) -> Result<()> {
    r.run_ok("pkexec", &["chfn", "-f", name, user]).map(|_| ())
}

pub fn set_locked(r: &dyn CommandRunner, user: &str, locked: bool) -> Result<()> {
    r.run_ok(
        "pkexec",
        &["passwd", if locked { "-l" } else { "-u" }, user],
    )
    .map(|_| ())
}
