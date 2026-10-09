//! A greetd-less login backend for systems without PAM (RustOS): accounts from `passwd`,
//! SHA-512 crypt hashes from `shadow` (the storage partition's copies first, as RustOS's own
//! `login` and `passwd` use them). On success the user name is written to the result file and
//! the greeter exits; the session script then starts the desktop as that user
//! (`edex-greeter run-as USER -- …`).

use std::path::{Path, PathBuf};

use anyhow::{anyhow, Context, Result};

use crate::greetd::Step;

/// Where accounts are looked up, first file that exists wins.
#[derive(Clone, Debug)]
pub struct Files {
    pub passwd: Vec<PathBuf>,
    pub shadow: Vec<PathBuf>,
}

impl Default for Files {
    fn default() -> Self {
        Self {
            passwd: vec!["/storage/etc/passwd".into(), "/etc/passwd".into()],
            shadow: vec!["/storage/etc/shadow".into(), "/etc/shadow".into()],
        }
    }
}

fn read_first(paths: &[PathBuf]) -> String {
    paths
        .iter()
        .find_map(|p| std::fs::read_to_string(p).ok())
        .unwrap_or_default()
}

/// An account from `passwd`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Account {
    pub name: String,
    pub uid: u32,
    pub gid: u32,
    pub home: String,
    pub shell: String,
}

pub fn account(files: &Files, user: &str) -> Option<Account> {
    read_first(&files.passwd).lines().find_map(|l| {
        let f: Vec<&str> = l.split(':').collect();
        (f.len() >= 7 && f[0] == user).then(|| Account {
            name: f[0].into(),
            uid: f[2].parse().unwrap_or(u32::MAX),
            gid: f[3].parse().unwrap_or(u32::MAX),
            home: if f[5].is_empty() {
                "/".into()
            } else {
                f[5].into()
            },
            shell: if f[6].is_empty() {
                "/bin/sh".into()
            } else {
                f[6].into()
            },
        })
    })
}

/// The shadow hash: `Some("")` for an account without a password, `None` for a locked or
/// unknown one.
fn shadow_hash(files: &Files, user: &str) -> Option<String> {
    let h = read_first(&files.shadow).lines().find_map(|l| {
        let mut f = l.split(':');
        (f.next() == Some(user)).then(|| f.next().unwrap_or("").to_string())
    })?;
    (!h.starts_with('!') && !h.starts_with('*')).then_some(h)
}

/// Check a password against a crypt(3) hash (SHA-512, `$6$`).
pub fn verify(password: &str, hash: &str) -> bool {
    if hash.starts_with("$6$") {
        sha_crypt::sha512_check(password, hash).is_ok()
    } else {
        false
    }
}

pub struct LocalLogin {
    files: Files,
    user: Option<String>,
    authenticated: Option<String>,
    result: Option<PathBuf>,
}

impl LocalLogin {
    pub fn new(files: Files, result: Option<PathBuf>) -> Self {
        Self {
            files,
            user: None,
            authenticated: None,
            result,
        }
    }

    pub fn create_session(&mut self, username: &str) -> Result<Step> {
        self.user = Some(username.to_string());
        self.authenticated = None;
        let known = account(&self.files, username).is_some();
        match shadow_hash(&self.files, username) {
            Some(h) if known && h.is_empty() => {
                self.authenticated = Some(username.to_string());
                Ok(Step::Success)
            }
            // Ask for a password even for unknown or locked accounts, so names are not revealed.
            _ => Ok(Step::Prompt {
                message: "Password:".into(),
                secret: true,
            }),
        }
    }

    pub fn respond(&mut self, answer: Option<String>) -> Result<Step> {
        let user = self
            .user
            .clone()
            .ok_or_else(|| anyhow!("no login in progress"))?;
        let ok = account(&self.files, &user).is_some()
            && shadow_hash(&self.files, &user)
                .is_some_and(|h| verify(answer.as_deref().unwrap_or(""), &h));
        if ok {
            self.authenticated = Some(user);
            Ok(Step::Success)
        } else {
            // Slow down guessing.
            std::thread::sleep(std::time::Duration::from_millis(800));
            self.user = None;
            Ok(Step::Failed("authentication failed".into()))
        }
    }

    /// Hand the authenticated user to the session script.
    pub fn start_session(&mut self, _cmd: Vec<String>, _env: Vec<String>) -> Result<()> {
        let user = self
            .authenticated
            .clone()
            .ok_or_else(|| anyhow!("not authenticated"))?;
        match &self.result {
            Some(path) => std::fs::write(path, format!("{user}\n"))
                .with_context(|| format!("writing {}", path.display()))?,
            None => println!("{user}"),
        }
        Ok(())
    }

    pub fn cancel(&mut self) -> Result<()> {
        self.user = None;
        self.authenticated = None;
        Ok(())
    }
}

/// `edex-greeter run-as USER -- CMD…`: become USER (gid, uid, HOME, USER, LOGNAME, SHELL,
/// working directory) and exec CMD. For session scripts running as root.
pub fn run_as(files: &Files, user: &str, cmd: &[String]) -> Result<std::convert::Infallible> {
    use std::os::unix::process::CommandExt;
    let a = account(files, user).ok_or_else(|| anyhow!("no such user: {user}"))?;
    let (prog, args) = cmd.split_first().ok_or_else(|| anyhow!("no command"))?;
    let mut c = std::process::Command::new(prog);
    c.args(args)
        .env("HOME", &a.home)
        .env("USER", &a.name)
        .env("LOGNAME", &a.name)
        .env("SHELL", &a.shell)
        .current_dir(if Path::new(&a.home).is_dir() {
            &a.home
        } else {
            "/"
        })
        .gid(a.gid)
        .uid(a.uid);
    Err(c.exec()).with_context(|| format!("running {prog} as {user}"))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn files(dir: &Path, passwd: &str, shadow: &str) -> Files {
        std::fs::write(dir.join("passwd"), passwd).unwrap();
        std::fs::write(dir.join("shadow"), shadow).unwrap();
        Files {
            passwd: vec![dir.join("missing"), dir.join("passwd")],
            shadow: vec![dir.join("shadow")],
        }
    }

    #[test]
    fn password_conversation() {
        let dir = tempfile::tempdir().unwrap();
        let params = sha_crypt::Sha512Params::new(5000).unwrap();
        let hash = sha_crypt::sha512_simple("hunter2", &params).unwrap();
        let f = files(
            dir.path(),
            "root:x:0:0:root:/root:/bin/sh\nari:x:1000:1000:Ari:/home/ari:/bin/sh\nlocked:x:1001:1001::/:/bin/sh\n",
            &format!("root::0:0:99999:7:::\nari:{hash}:0:0:99999:7:::\nlocked:!{hash}:0::::::\n"),
        );
        let result = dir.path().join("user");
        let mut l = LocalLogin::new(f.clone(), Some(result.clone()));
        // No password: straight in.
        assert_eq!(l.create_session("root").unwrap(), Step::Success);
        // Wrong, then right password.
        assert!(matches!(
            l.create_session("ari").unwrap(),
            Step::Prompt { secret: true, .. }
        ));
        assert!(matches!(
            l.respond(Some("nope".into())).unwrap(),
            Step::Failed(_)
        ));
        l.create_session("ari").unwrap();
        assert_eq!(l.respond(Some("hunter2".into())).unwrap(), Step::Success);
        l.start_session(vec![], vec![]).unwrap();
        assert_eq!(std::fs::read_to_string(&result).unwrap(), "ari\n");
        // Locked and unknown accounts get a prompt and then fail.
        for u in ["locked", "nobody"] {
            let mut l = LocalLogin::new(f.clone(), None);
            assert!(matches!(l.create_session(u).unwrap(), Step::Prompt { .. }));
            assert!(matches!(
                l.respond(Some("hunter2".into())).unwrap(),
                Step::Failed(_)
            ));
        }
        assert!(!verify("x", "$1$md5$abc"));
    }
}
