//! Local user accounts. RustOS keeps edited account files on `/storage/etc`; changes go
//! through the `edex-auth` helper (there is no PAM, chfn or pkexec).

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

/// The first account file that exists: `/storage/etc/X`, then `/etc/X`.
fn account_file(name: &str) -> String {
    [format!("/storage/etc/{name}"), format!("/etc/{name}")]
        .iter()
        .find_map(|p| std::fs::read_to_string(p).ok())
        .unwrap_or_default()
}

pub fn query(r: &dyn CommandRunner) -> Vec<UserInfo> {
    parse(r, &account_file("passwd"), &account_file("group"))
}

fn parse(r: &dyn CommandRunner, passwd: &str, group: &str) -> Vec<UserInfo> {
    let wheel: Vec<String> = Some(group)
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
            // root is the RustOS console account; regular users start at 1000.
            let person = uid == 0 || (1000..60000).contains(&uid);
            if !person || f[6].ends_with("nologin") || f[6].ends_with("false") {
                return None;
            }
            Some(UserInfo {
                name: f[0].into(),
                real_name: f[4].split(',').next().unwrap_or("").into(),
                uid,
                admin: uid == 0 || wheel.iter().any(|w| w == f[0]),
                locked: false,
                shell: f[6].into(),
                home: f[5].into(),
            })
        })
        .collect();
    for u in &mut users {
        // `edex-auth status NAME` prints locked | usable | nopassword.
        if let Ok(out) = r.run("edex-auth", &["status", &u.name]) {
            if out.ok() {
                u.locked = out.stdout.trim() == "locked";
            }
        }
    }
    users
}

pub fn set_real_name(r: &dyn CommandRunner, user: &str, name: &str) -> Result<()> {
    r.run_ok("edex-auth", &["chfn", user, name]).map(|_| ())
}

pub fn set_locked(r: &dyn CommandRunner, user: &str, locked: bool) -> Result<()> {
    r.run_ok("edex-auth", &[if locked { "lock" } else { "unlock" }, user])
        .map(|_| ())
}

/// Change a password: `edex-auth passwd USER` reads the old and new password from stdin.
pub fn set_password(r: &dyn CommandRunner, user: &str, old: &str, new: &str) -> Result<()> {
    let out = r.run_with_stdin("edex-auth", &["passwd", user], &format!("{old}\n{new}\n"))?;
    if out.ok() {
        Ok(())
    } else {
        Err(anyhow::anyhow!("{}", out.stderr.trim()))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::runner::FakeRunner;

    #[test]
    fn lists_people_with_lock_state() {
        let passwd = "root:x:0:0:root:/root:/bin/sh\n\
            daemon:x:2:2::/:/sbin/nologin\n\
            ari:x:1000:1000:Ari C,,,:/home/ari:/bin/sh\n\
            guest:x:1001:1001::/home/guest:/bin/sh\n";
        let group = "wheel:x:10:ari\n";
        let r = FakeRunner::default()
            .with("edex-auth status guest", "locked\n")
            .with("edex-auth status ari", "usable\n")
            .with("edex-auth status root", "nopassword\n");
        let users = parse(&r, passwd, group);
        let names: Vec<_> = users.iter().map(|u| u.name.as_str()).collect();
        assert_eq!(names, ["root", "ari", "guest"]);
        assert!(users[0].admin && users[1].admin && !users[2].admin);
        assert_eq!(users[1].real_name, "Ari C");
        assert!(users[2].locked && !users[1].locked);
        set_locked(&r, "guest", false).unwrap_or(());
        assert!(r.calls().contains(&"edex-auth unlock guest".to_string()));
    }
}
