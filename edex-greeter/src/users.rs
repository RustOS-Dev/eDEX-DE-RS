//! Login users from /etc/passwd.

use std::path::Path;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct User {
    pub name: String,
    pub real_name: String,
    pub uid: u32,
}

pub fn parse_passwd(text: &str, min_uid: u32) -> Vec<User> {
    let mut users: Vec<User> = text
        .lines()
        .filter_map(|l| {
            let f: Vec<&str> = l.split(':').collect();
            if f.len() < 7 {
                return None;
            }
            let uid: u32 = f[2].parse().ok()?;
            let shell = f[6].trim();
            if uid < min_uid
                || uid >= 60000
                || shell.ends_with("nologin")
                || shell.ends_with("/false")
                || shell.is_empty()
            {
                return None;
            }
            let real = f[4].split(',').next().unwrap_or("").trim();
            Some(User {
                name: f[0].to_string(),
                real_name: if real.is_empty() {
                    f[0].to_string()
                } else {
                    real.to_string()
                },
                uid,
            })
        })
        .collect();
    users.sort_by_key(|u| u.uid);
    users
}

pub fn load(path: &Path, min_uid: u32) -> Vec<User> {
    std::fs::read_to_string(path)
        .map(|t| parse_passwd(&t, min_uid))
        .unwrap_or_default()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn filters_system_accounts() {
        let passwd = "root:x:0:0:root:/root:/bin/bash\n\
                      daemon:x:1:1::/:/usr/sbin/nologin\n\
                      ari:x:1000:1000:Ari Cummings,,,:/home/ari:/usr/bin/fish\n\
                      liveuser:x:1001:1001::/home/liveuser:/bin/bash\n\
                      nobody:x:65534:65534::/:/usr/bin/nologin\n";
        let u = parse_passwd(passwd, 1000);
        assert_eq!(u.len(), 2);
        assert_eq!(u[0].real_name, "Ari Cummings");
        assert_eq!(u[1].real_name, "liveuser");
    }
}
