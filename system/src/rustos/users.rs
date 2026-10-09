//! Users on RustOS: `/etc/passwd` (root and uid ≥ 1000) with locked accounts from the shadow
//! file (`/storage/etc/shadow`, written by `passwd`, else `/etc/shadow`).

use std::path::Path;

use crate::users::UserInfo;

pub fn parse(passwd: &str, shadow: &str, group: &str) -> Vec<UserInfo> {
    let wheel: Vec<&str> = group
        .lines()
        .find(|l| l.starts_with("wheel:"))
        .and_then(|l| l.rsplit(':').next())
        .map(|m| m.split(',').collect())
        .unwrap_or_default();
    passwd
        .lines()
        .filter_map(|l| {
            let f: Vec<&str> = l.split(':').collect();
            if f.len() < 7 {
                return None;
            }
            let uid: u32 = f[2].parse().ok()?;
            if !(uid == 0 || (1000..60000).contains(&uid))
                || f[6].ends_with("nologin")
                || f[6].ends_with("false")
            {
                return None;
            }
            let locked = shadow
                .lines()
                .find_map(|s| s.strip_prefix(&format!("{}:", f[0])))
                .map(|rest| rest.starts_with('!') || rest.starts_with('*'))
                .unwrap_or(false);
            Some(UserInfo {
                name: f[0].into(),
                real_name: f[4].split(',').next().unwrap_or("").into(),
                uid,
                admin: uid == 0 || wheel.contains(&f[0]),
                locked,
                shell: f[6].into(),
                home: f[5].into(),
            })
        })
        .collect()
}

pub fn query() -> Vec<UserInfo> {
    let read = |p: &str| std::fs::read_to_string(Path::new(p)).unwrap_or_default();
    let shadow =
        std::fs::read_to_string("/storage/etc/shadow").unwrap_or_else(|_| read("/etc/shadow"));
    parse(&read("/etc/passwd"), &shadow, &read("/etc/group"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn root_and_regular_users() {
        let passwd = "root:x:0:0:root:/root:/bin/sh\ndaemon:x:2:2::/:/bin/false\nari:x:1000:1000:Ari C,,,:/home/ari:/bin/sh\n";
        let shadow = "root:$6$abc$def:19000::::::\nari:!:19000::::::\n";
        let u = parse(passwd, shadow, "wheel:x:10:ari\n");
        assert_eq!(u.len(), 2);
        assert_eq!(u[0].name, "root");
        assert!(u[0].admin && !u[0].locked);
        assert_eq!(u[1].real_name, "Ari C");
        assert!(u[1].admin && u[1].locked);
    }
}
