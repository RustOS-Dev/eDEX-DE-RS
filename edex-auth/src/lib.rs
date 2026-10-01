//! Account files the way RustOS's `login` and `passwd` use them: `/storage/etc/{passwd,shadow}`
//! (the persistent partition) take precedence over `/etc`, edits are written to `/storage/etc`,
//! hashes are SHA-512 crypt (`$6$`), an empty hash means "no password" and a hash starting with
//! `!` or `*` means the account is locked.

use std::{
    fs,
    io::Write,
    os::unix::fs::{OpenOptionsExt, PermissionsExt},
    path::{Path, PathBuf},
};

use anyhow::{anyhow, bail, Context, Result};
use sha_crypt::{PasswordHasher, PasswordVerifier, ShaCrypt};

/// Where account files live; tests point it at a temporary directory.
#[derive(Clone, Debug)]
pub struct Accounts {
    /// Persistent copies (`/storage/etc`), read first and written to.
    pub store: PathBuf,
    /// The image's defaults (`/etc`).
    pub base: PathBuf,
}

impl Default for Accounts {
    fn default() -> Self {
        Self {
            store: PathBuf::from("/storage/etc"),
            base: PathBuf::from("/etc"),
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Status {
    Usable,
    Locked,
    NoPassword,
}

impl Status {
    pub fn as_str(&self) -> &'static str {
        match self {
            Status::Usable => "usable",
            Status::Locked => "locked",
            Status::NoPassword => "nopassword",
        }
    }
}

impl Accounts {
    fn read(&self, name: &str) -> String {
        fs::read_to_string(self.store.join(name))
            .or_else(|_| fs::read_to_string(self.base.join(name)))
            .unwrap_or_default()
    }

    /// The account's passwd line fields, if it exists.
    pub fn user(&self, user: &str) -> Option<Vec<String>> {
        self.read("passwd").lines().find_map(|l| {
            let f: Vec<String> = l.split(':').map(str::to_string).collect();
            (f.len() >= 7 && f[0] == user).then_some(f)
        })
    }

    pub fn uid(&self, user: &str) -> Option<u32> {
        self.user(user)?.get(2)?.parse().ok()
    }

    /// The shadow hash, `None` when the user has no shadow entry.
    pub fn hash(&self, user: &str) -> Option<String> {
        self.read("shadow").lines().find_map(|l| {
            let mut f = l.split(':');
            (f.next() == Some(user)).then(|| f.next().unwrap_or("").to_string())
        })
    }

    pub fn status(&self, user: &str) -> Result<Status> {
        self.user(user)
            .ok_or_else(|| anyhow!("unknown user {user}"))?;
        Ok(match self.hash(user) {
            None => Status::NoPassword,
            Some(h) if h.is_empty() => Status::NoPassword,
            Some(h) if h.starts_with('!') || h.starts_with('*') => Status::Locked,
            Some(_) => Status::Usable,
        })
    }

    /// True when `password` opens `user`'s account (a password-less account accepts any).
    pub fn check(&self, user: &str, password: &str) -> bool {
        if self.user(user).is_none() {
            // Spend the same time on unknown names so they are not revealed.
            let _ = hash_password(password);
            return false;
        }
        match self.hash(user) {
            None => true,
            Some(h) if h.is_empty() => true,
            Some(h) if h.starts_with('!') || h.starts_with('*') => false,
            Some(h) => verify(password, &h),
        }
    }

    /// Replace the shadow hash of `user` (creating the entry when needed).
    pub fn set_hash(&self, user: &str, hash: &str) -> Result<()> {
        self.user(user)
            .ok_or_else(|| anyhow!("unknown user {user}"))?;
        let mut found = false;
        let mut out = String::new();
        for l in self.read("shadow").lines() {
            let mut f: Vec<String> = l.split(':').map(str::to_string).collect();
            if f.first().map(String::as_str) == Some(user) {
                f.resize(f.len().max(2), String::new());
                f[1] = hash.to_string();
                found = true;
            }
            out.push_str(&f.join(":"));
            out.push('\n');
        }
        if !found {
            out.push_str(&format!("{user}:{hash}:0:0:99999:7:::\n"));
        }
        self.write("shadow", &out, 0o600)
    }

    pub fn set_password(&self, user: &str, password: &str) -> Result<()> {
        let hash = if password.is_empty() {
            String::new()
        } else {
            hash_password(password)?
        };
        self.set_hash(user, &hash)
    }

    pub fn set_locked(&self, user: &str, locked: bool) -> Result<()> {
        let hash = self.hash(user).unwrap_or_default();
        let bare = hash.trim_start_matches('!');
        let new = if locked {
            format!("!{bare}")
        } else {
            bare.to_string()
        };
        self.set_hash(user, &new)
    }

    /// Set the full name (first GECOS field), keeping the other fields.
    pub fn set_real_name(&self, user: &str, name: &str) -> Result<()> {
        if name.contains([':', ',', '\n']) {
            bail!("a name cannot contain ':', ',' or a line break");
        }
        let mut found = false;
        let mut out = String::new();
        for l in self.read("passwd").lines() {
            let mut f: Vec<String> = l.split(':').map(str::to_string).collect();
            if f.len() >= 7 && f[0] == user {
                let mut gecos: Vec<String> = f[4].split(',').map(str::to_string).collect();
                gecos[0] = name.to_string();
                f[4] = gecos.join(",");
                found = true;
            }
            out.push_str(&f.join(":"));
            out.push('\n');
        }
        if !found {
            bail!("unknown user {user}");
        }
        self.write("passwd", &out, 0o644)
    }

    /// Write `name` to the persistent store atomically.
    fn write(&self, name: &str, content: &str, mode: u32) -> Result<()> {
        fs::create_dir_all(&self.store)
            .with_context(|| format!("creating {}", self.store.display()))?;
        let path = self.store.join(name);
        let tmp = self.store.join(format!(".{name}.edex-auth"));
        {
            let mut f = fs::OpenOptions::new()
                .write(true)
                .create(true)
                .truncate(true)
                .mode(mode)
                .open(&tmp)
                .with_context(|| format!("writing {}", tmp.display()))?;
            f.write_all(content.as_bytes())?;
            f.sync_all()?;
        }
        fs::set_permissions(&tmp, fs::Permissions::from_mode(mode))?;
        fs::rename(&tmp, &path).with_context(|| format!("replacing {}", path.display()))?;
        Ok(())
    }
}

pub fn verify(password: &str, hash: &str) -> bool {
    ShaCrypt::SHA512
        .verify_password(password.as_bytes(), hash)
        .is_ok()
}

pub fn hash_password(password: &str) -> Result<String> {
    let mut salt = [0u8; 12];
    random(&mut salt)?;
    let h = ShaCrypt::SHA512
        .hash_password_with_salt(password.as_bytes(), &salt)
        .map_err(|e| anyhow!("hashing: {e}"))?;
    Ok(h.to_string())
}

fn random(buf: &mut [u8]) -> Result<()> {
    // SAFETY: getrandom writes at most buf.len() bytes into buf.
    let n = unsafe { libc::getrandom(buf.as_mut_ptr().cast(), buf.len(), 0) };
    if n as usize == buf.len() {
        return Ok(());
    }
    let mut f = fs::File::open("/dev/urandom").context("no randomness")?;
    std::io::Read::read_exact(&mut f, buf)?;
    Ok(())
}

/// Failed attempts are counted per user in `dir` so repeated guesses slow down even across
/// processes.
pub fn failure_delay(dir: &Path, user: &str, failed: bool) -> std::time::Duration {
    let path = dir.join(user.replace('/', "_"));
    let count: u32 = fs::read_to_string(&path)
        .ok()
        .and_then(|s| s.trim().parse().ok())
        .unwrap_or(0);
    if failed {
        let _ = fs::create_dir_all(dir);
        let _ = fs::write(&path, (count + 1).to_string());
        std::time::Duration::from_secs(2u64.saturating_pow(count.min(5)).min(30))
    } else {
        let _ = fs::remove_file(&path);
        std::time::Duration::ZERO
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn accounts() -> (tempfile::TempDir, Accounts) {
        let dir = tempfile::tempdir().unwrap();
        let base = dir.path().join("etc");
        fs::create_dir_all(&base).unwrap();
        fs::write(
            base.join("passwd"),
            "root:x:0:0:root:/root:/bin/sh\nari:x:1000:1000:Ari,,,:/home/ari:/bin/sh\n",
        )
        .unwrap();
        // root has no password, as on a fresh RustOS image.
        fs::write(base.join("shadow"), "root::0:0:99999:7:::\n").unwrap();
        let a = Accounts {
            store: dir.path().join("storage/etc"),
            base,
        };
        (dir, a)
    }

    #[test]
    fn verifies_rustos_style_hashes() {
        // The SHA-crypt specification's vector, in the format RustOS's passwd writes.
        let h = "$6$saltstring$svn8UoSVapNtMuq1ukKS4tPQd8iKwSMHWjl/O817G3uBnIFNjnQJuesI68u4OTLiBFdcbYEdFCoEOfaS35inz1";
        assert!(verify("Hello world!", h));
        assert!(!verify("hello", h));
        let ours = hash_password("correct horse").unwrap();
        assert!(ours.starts_with("$6$rounds=5000$"));
        assert!(verify("correct horse", &ours));
        assert!(!verify("wrong", &ours));
    }

    #[test]
    fn account_lifecycle() {
        let (_dir, a) = accounts();
        assert_eq!(a.status("root").unwrap(), Status::NoPassword);
        assert!(a.check("root", "anything"));
        assert!(!a.check("nobody", "x"));
        assert_eq!(a.uid("ari"), Some(1000));
        // ari has no shadow entry yet: password-less until one is set.
        a.set_password("ari", "s3cret").unwrap();
        assert_eq!(a.status("ari").unwrap(), Status::Usable);
        assert!(a.check("ari", "s3cret") && !a.check("ari", "nope"));
        // Edits land on the storage partition; root's entry is carried over.
        let stored = fs::read_to_string(a.store.join("shadow")).unwrap();
        assert!(stored.starts_with("root::"));
        let mode = fs::metadata(a.store.join("shadow"))
            .unwrap()
            .permissions()
            .mode();
        assert_eq!(mode & 0o777, 0o600);
        a.set_locked("ari", true).unwrap();
        assert_eq!(a.status("ari").unwrap(), Status::Locked);
        assert!(!a.check("ari", "s3cret"));
        a.set_locked("ari", false).unwrap();
        assert!(a.check("ari", "s3cret"));
        a.set_real_name("ari", "Ari Cummings").unwrap();
        assert_eq!(a.user("ari").unwrap()[4], "Ari Cummings,,,");
        assert!(a.set_real_name("ari", "bad:name").is_err());
        assert!(a.set_password("ghost", "x").is_err());
    }

    #[test]
    fn failures_slow_down() {
        let dir = tempfile::tempdir().unwrap();
        assert_eq!(failure_delay(dir.path(), "ari", true).as_secs(), 1);
        assert_eq!(failure_delay(dir.path(), "ari", true).as_secs(), 2);
        assert_eq!(failure_delay(dir.path(), "ari", true).as_secs(), 4);
        assert_eq!(failure_delay(dir.path(), "ari", false).as_secs(), 0);
        assert_eq!(failure_delay(dir.path(), "ari", true).as_secs(), 1);
    }
}
