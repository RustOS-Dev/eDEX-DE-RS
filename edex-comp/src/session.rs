//! The session: who it belongs to, its environment, and the supervised programs that make it up
//! (the session D-Bus, PipeWire, the eDEX shell, portals). edex-comp replaces the systemd user
//! units and greetd of the Linux build: it starts them, restarts the ones that crash, and runs
//! everything a binding or the shell launches with the same environment.

use std::{
    collections::HashMap,
    ffi::CString,
    io,
    os::unix::process::CommandExt,
    path::{Path, PathBuf},
    process::{Child, Command, Stdio},
    time::{Duration, Instant},
};

use tracing::{info, warn};

/// A user account from `/etc/passwd`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct User {
    pub name: String,
    pub uid: u32,
    pub gid: u32,
    pub gecos: String,
    pub home: PathBuf,
    pub shell: String,
}

impl User {
    pub fn display_name(&self) -> &str {
        let full = self.gecos.split(',').next().unwrap_or("").trim();
        if full.is_empty() {
            &self.name
        } else {
            full
        }
    }

    pub fn runtime_dir(&self) -> PathBuf {
        PathBuf::from(format!("/run/user/{}", self.uid))
    }

    pub fn config_path(&self) -> PathBuf {
        self.home.join(".config/edex-de/config.toml")
    }
}

pub fn parse_passwd(text: &str) -> Vec<User> {
    text.lines()
        .filter(|l| !l.trim().is_empty() && !l.starts_with('#'))
        .filter_map(|l| {
            let f: Vec<&str> = l.split(':').collect();
            if f.len() < 7 {
                return None;
            }
            Some(User {
                name: f[0].to_string(),
                uid: f[2].parse().ok()?,
                gid: f[3].parse().ok()?,
                gecos: f[4].to_string(),
                home: PathBuf::from(f[5]),
                shell: f[6].to_string(),
            })
        })
        .collect()
}

/// RustOS keeps edited account files on the persistent `/storage` partition.
pub const PASSWD_PATHS: [&str; 2] = ["/storage/etc/passwd", "/etc/passwd"];

pub fn lookup_user(name: &str) -> Option<User> {
    PASSWD_PATHS
        .iter()
        .filter_map(|p| std::fs::read_to_string(p).ok())
        .flat_map(|t| parse_passwd(&t))
        .find(|u| u.name == name)
}

pub fn lookup_uid(uid: u32) -> Option<User> {
    PASSWD_PATHS
        .iter()
        .filter_map(|p| std::fs::read_to_string(p).ok())
        .flat_map(|t| parse_passwd(&t))
        .find(|u| u.uid == uid)
}

/// The user this process runs as (the session user when edex-comp is started without the
/// greeter, e.g. nested for development).
pub fn current_user() -> User {
    // SAFETY: getuid/getgid have no preconditions.
    let (uid, gid) = unsafe { (libc::getuid(), libc::getgid()) };
    lookup_uid(uid).unwrap_or_else(|| User {
        name: std::env::var("USER").unwrap_or_else(|_| "root".into()),
        uid,
        gid,
        gecos: String::new(),
        home: std::env::var("HOME")
            .map(PathBuf::from)
            .unwrap_or_else(|_| PathBuf::from("/root")),
        shell: "/bin/sh".into(),
    })
}

/// Create `/run/user/UID` owned by the user, mode 0700.
pub fn ensure_runtime_dir(user: &User) -> io::Result<PathBuf> {
    use std::os::unix::fs::PermissionsExt;
    let dir = user.runtime_dir();
    std::fs::create_dir_all(&dir)?;
    std::fs::set_permissions(&dir, std::fs::Permissions::from_mode(0o700))?;
    chown(&dir, user.uid, user.gid)?;
    Ok(dir)
}

pub fn chown(path: &Path, uid: u32, gid: u32) -> io::Result<()> {
    // SAFETY: getuid has no preconditions.
    if unsafe { libc::getuid() } != 0 {
        return Ok(());
    }
    std::os::unix::fs::chown(path, Some(uid), Some(gid))
}

/// The environment every session program gets.
pub fn session_env(
    user: &User,
    runtime_dir: &Path,
    wayland_display: Option<&str>,
    x_display: Option<u32>,
    comp_socket: &Path,
) -> Vec<(String, String)> {
    let mut env = vec![
        ("HOME".into(), user.home.display().to_string()),
        ("USER".into(), user.name.clone()),
        ("LOGNAME".into(), user.name.clone()),
        ("SHELL".into(), user.shell.clone()),
        (
            "PATH".into(),
            "/usr/local/bin:/usr/bin:/bin:/usr/local/sbin:/usr/sbin:/sbin".into(),
        ),
        ("XDG_RUNTIME_DIR".into(), runtime_dir.display().to_string()),
        ("XDG_SESSION_TYPE".into(), "wayland".into()),
        ("XDG_SESSION_DESKTOP".into(), "eDEX-DE".into()),
        ("XDG_CURRENT_DESKTOP".into(), "eDEX-DE".into()),
        (
            "DBUS_SESSION_BUS_ADDRESS".into(),
            format!("unix:path={}/bus", runtime_dir.display()),
        ),
        (
            comp_proto::SOCKET_ENV.into(),
            comp_socket.display().to_string(),
        ),
        ("MOZ_ENABLE_WAYLAND".into(), "1".into()),
        ("QT_QPA_PLATFORM".into(), "wayland;xcb".into()),
        ("GDK_BACKEND".into(), "wayland,x11".into()),
        ("SDL_VIDEODRIVER".into(), "wayland".into()),
        ("_JAVA_AWT_WM_NONREPARENTING".into(), "1".into()),
    ];
    if let Some(w) = wayland_display {
        env.push(("WAYLAND_DISPLAY".into(), w.into()));
    }
    if let Some(x) = x_display {
        env.push(("DISPLAY".into(), format!(":{x}")));
    }
    // Keep locale and terminal settings from the compositor's own environment.
    for key in ["LANG", "LC_ALL", "TZ", "TERM", "EDEX_SHARE_DIR", "RUST_LOG"] {
        if let Ok(v) = std::env::var(key) {
            env.push((key.into(), v));
        }
    }
    env
}

/// `sh -c command` in its own session, as `user` when edex-comp runs as root.
pub fn command(user: &User, env: &[(String, String)], command: &str) -> Command {
    let mut cmd = Command::new("/bin/sh");
    cmd.arg("-c").arg(command);
    prepare(&mut cmd, user, env);
    cmd
}

/// Run `program args…` directly (no shell).
pub fn program(user: &User, env: &[(String, String)], program: &str, args: &[&str]) -> Command {
    let mut cmd = Command::new(program);
    cmd.args(args);
    prepare(&mut cmd, user, env);
    cmd
}

fn prepare(cmd: &mut Command, user: &User, env: &[(String, String)]) {
    // A home directory that does not exist (RustOS's root account) must not stop the program.
    let dir = if user.home.is_dir() {
        user.home.as_path()
    } else {
        Path::new("/")
    };
    cmd.env_clear()
        .envs(env.iter().map(|(k, v)| (k.as_str(), v.as_str())))
        .current_dir(dir)
        .stdin(Stdio::null());
    let (uid, gid) = (user.uid, user.gid);
    let name = CString::new(user.name.clone()).unwrap_or_default();
    // SAFETY: only async-signal-safe libc calls between fork and exec.
    unsafe {
        cmd.pre_exec(move || {
            if libc::setsid() < 0 {
                // Already a session leader is fine.
            }
            if libc::getuid() == 0
                && uid != 0
                && (libc::initgroups(name.as_ptr(), gid as _) != 0
                    || libc::setgid(gid) != 0
                    || libc::setuid(uid) != 0)
            {
                return Err(io::Error::last_os_error());
            }
            // Children should not inherit the compositor's signal mask.
            let mut set: libc::sigset_t = std::mem::zeroed();
            libc::sigemptyset(&mut set);
            libc::sigprocmask(libc::SIG_SETMASK, &set, std::ptr::null_mut());
            Ok(())
        });
    }
}

/// Restart policy of a supervised program.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Restart {
    Never,
    /// Restart after a non-zero exit or a signal.
    OnFailure,
    Always,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ServiceSpec {
    pub name: String,
    pub command: String,
    pub restart: Restart,
    /// Skip silently when the program is not installed.
    pub optional_binary: Option<String>,
}

struct Service {
    spec: ServiceSpec,
    child: Option<Child>,
    failures: Vec<Instant>,
    next_start: Option<Instant>,
    gave_up: bool,
}

/// Starts and restarts the session's programs. Call [`Supervisor::reap`] on SIGCHLD and
/// [`Supervisor::tick`] once a second.
pub struct Supervisor {
    user: User,
    env: Vec<(String, String)>,
    services: Vec<Service>,
    /// One-shot children (bindings, `exec`): reaped, not restarted.
    detached: Vec<Child>,
}

impl Supervisor {
    pub fn new(user: User, env: Vec<(String, String)>) -> Self {
        Self {
            user,
            env,
            services: Vec::new(),
            detached: Vec::new(),
        }
    }

    pub fn user(&self) -> &User {
        &self.user
    }

    pub fn env(&self) -> &[(String, String)] {
        &self.env
    }

    pub fn set_env(&mut self, key: &str, value: &str) {
        self.env.retain(|(k, _)| k != key);
        self.env.push((key.into(), value.into()));
    }

    pub fn add(&mut self, spec: ServiceSpec) {
        if let Some(bin) = &spec.optional_binary {
            if which(bin).is_none() {
                info!(service = spec.name, "{bin} is not installed; not starting");
                return;
            }
        }
        let mut s = Service {
            spec,
            child: None,
            failures: Vec::new(),
            next_start: None,
            gave_up: false,
        };
        start(&mut s, &self.user, &self.env);
        self.services.push(s);
    }

    pub fn is_running(&self, name: &str) -> bool {
        self.services
            .iter()
            .any(|s| s.spec.name == name && s.child.is_some())
    }

    /// Start a one-shot program (`sh -c`).
    pub fn spawn(&mut self, cmd: &str, extra_env: &[(String, String)]) -> io::Result<u32> {
        let mut env = self.env.clone();
        for (k, v) in extra_env {
            env.retain(|(ek, _)| ek != k);
            env.push((k.clone(), v.clone()));
        }
        let child = command(&self.user, &env, cmd)
            .stdout(Stdio::null())
            .stderr(Stdio::inherit())
            .spawn()?;
        let pid = child.id();
        self.detached.push(child);
        Ok(pid)
    }

    /// Collect exited children and schedule restarts.
    pub fn reap(&mut self) {
        self.detached
            .retain_mut(|c| matches!(c.try_wait(), Ok(None)));
        let now = Instant::now();
        for s in &mut self.services {
            let Some(child) = s.child.as_mut() else {
                continue;
            };
            let status = match child.try_wait() {
                Ok(Some(status)) => status,
                Ok(None) => continue,
                Err(e) => {
                    warn!(service = s.spec.name, "wait failed: {e}");
                    continue;
                }
            };
            s.child = None;
            let failed = !status.success();
            warn!(service = s.spec.name, %status, "session program exited");
            let restart = match s.spec.restart {
                Restart::Never => false,
                Restart::OnFailure => failed,
                Restart::Always => true,
            };
            if !restart {
                continue;
            }
            s.failures
                .retain(|t| now.duration_since(*t) < Duration::from_secs(60));
            s.failures.push(now);
            if s.failures.len() > 5 {
                warn!(
                    service = s.spec.name,
                    "crashed 5 times in a minute; giving up"
                );
                s.gave_up = true;
                continue;
            }
            let backoff = Duration::from_secs(1 << (s.failures.len() - 1).min(4));
            s.next_start = Some(now + backoff);
        }
    }

    /// Start services whose restart delay has passed.
    pub fn tick(&mut self) {
        let now = Instant::now();
        for s in &mut self.services {
            if s.child.is_none() && !s.gave_up && s.next_start.is_some_and(|t| t <= now) {
                s.next_start = None;
                start(s, &self.user, &self.env);
            }
        }
    }

    /// Stop a service and start it again right away.
    pub fn restart(&mut self, name: &str) -> Result<(), String> {
        let s = self
            .services
            .iter_mut()
            .find(|s| s.spec.name == name)
            .ok_or_else(|| format!("no session program named {name}"))?;
        if let Some(mut child) = s.child.take() {
            let _ = child.kill();
            let _ = child.wait();
        }
        s.gave_up = false;
        s.failures.clear();
        s.next_start = None;
        start(s, &self.user, &self.env);
        Ok(())
    }

    /// Ask the session's programs to quit (SIGTERM), then forget them.
    pub fn shutdown(&mut self) {
        for s in &mut self.services {
            s.spec.restart = Restart::Never;
            if let Some(child) = &s.child {
                // SAFETY: kill with a pid we own.
                unsafe {
                    libc::kill(child.id() as i32, libc::SIGTERM);
                }
            }
        }
        for c in &self.detached {
            // Detached programs run in their own sessions; SIGHUP ends terminals and shells.
            unsafe {
                libc::kill(c.id() as i32, libc::SIGHUP);
            }
        }
    }

    pub fn status(&self) -> HashMap<String, bool> {
        self.services
            .iter()
            .map(|s| (s.spec.name.clone(), s.child.is_some()))
            .collect()
    }
}

fn start(s: &mut Service, user: &User, env: &[(String, String)]) {
    match command(user, env, &s.spec.command)
        .stdout(Stdio::null())
        .stderr(Stdio::inherit())
        .spawn()
    {
        Ok(child) => {
            info!(service = s.spec.name, pid = child.id(), "started");
            s.child = Some(child);
        }
        Err(e) => {
            warn!(
                service = s.spec.name,
                "could not start `{}`: {e}", s.spec.command
            );
            s.gave_up = true;
        }
    }
}

/// Find a program on the session PATH.
pub fn which(program: &str) -> Option<PathBuf> {
    if program.contains('/') {
        let p = PathBuf::from(program);
        return p.exists().then_some(p);
    }
    [
        "/usr/local/bin",
        "/usr/bin",
        "/bin",
        "/usr/local/sbin",
        "/usr/sbin",
        "/sbin",
    ]
    .iter()
    .map(|d| Path::new(d).join(program))
    .find(|p| p.exists())
}

/// The programs that make up an eDEX session, in start order.
pub fn default_services() -> Vec<ServiceSpec> {
    let spec = |name: &str, command: &str, restart, bin: Option<&str>| ServiceSpec {
        name: name.into(),
        command: command.into(),
        restart,
        optional_binary: bin.map(str::to_string),
    };
    vec![
        spec(
            "dbus",
            "exec dbus-daemon --session --nofork --nopidfile --address=\"$DBUS_SESSION_BUS_ADDRESS\"",
            Restart::OnFailure,
            Some("dbus-daemon"),
        ),
        spec("pipewire", "exec pipewire", Restart::OnFailure, Some("pipewire")),
        spec(
            "wireplumber",
            "sleep 1; exec wireplumber",
            Restart::OnFailure,
            Some("wireplumber"),
        ),
        spec(
            "pipewire-pulse",
            "exec pipewire-pulse",
            Restart::OnFailure,
            Some("pipewire-pulse"),
        ),
        spec(
            "portal",
            "sleep 1; exec /usr/libexec/xdg-desktop-portal -r",
            Restart::OnFailure,
            Some("/usr/libexec/xdg-desktop-portal"),
        ),
        spec(
            "portal-wlr",
            "sleep 1; exec /usr/libexec/xdg-desktop-portal-wlr",
            Restart::OnFailure,
            Some("/usr/libexec/xdg-desktop-portal-wlr"),
        ),
        spec(
            "cliphist",
            "exec wl-paste --watch cliphist store",
            Restart::OnFailure,
            Some("cliphist"),
        ),
        spec("edex-de", "exec edex-de run", Restart::Always, None),
    ]
}

#[cfg(test)]
mod tests {
    use super::*;

    const PASSWD: &str = "root:x:0:0:root:/root:/bin/sh\n\
        # comment\n\
        ari:x:1000:1000:Ari Cummings,,,:/home/ari:/bin/sh\n\
        broken:x:notanumber:0::/:/bin/sh\n";

    #[test]
    fn parses_passwd() {
        let users = parse_passwd(PASSWD);
        assert_eq!(users.len(), 2);
        assert_eq!(users[1].uid, 1000);
        assert_eq!(users[1].display_name(), "Ari Cummings");
        assert_eq!(users[0].display_name(), "root");
        assert_eq!(users[1].runtime_dir(), PathBuf::from("/run/user/1000"));
        assert_eq!(
            users[1].config_path(),
            PathBuf::from("/home/ari/.config/edex-de/config.toml")
        );
    }

    #[test]
    fn builds_the_session_environment() {
        let u = parse_passwd(PASSWD).remove(1);
        let env = session_env(
            &u,
            Path::new("/run/user/1000"),
            Some("wayland-1"),
            Some(0),
            Path::new("/run/user/1000/edex-comp.sock"),
        );
        let get = |k: &str| env.iter().find(|(ek, _)| ek == k).map(|(_, v)| v.as_str());
        assert_eq!(get("WAYLAND_DISPLAY"), Some("wayland-1"));
        assert_eq!(get("DISPLAY"), Some(":0"));
        assert_eq!(
            get("DBUS_SESSION_BUS_ADDRESS"),
            Some("unix:path=/run/user/1000/bus")
        );
        assert_eq!(
            get(comp_proto::SOCKET_ENV),
            Some("/run/user/1000/edex-comp.sock")
        );
        assert_eq!(get("HOME"), Some("/home/ari"));
    }

    #[test]
    fn supervisor_restarts_failures_and_reaps() {
        let me = current_user();
        let env = vec![("PATH".to_string(), "/usr/bin:/bin".to_string())];
        let mut sup = Supervisor::new(me, env);
        sup.add(ServiceSpec {
            name: "fails".into(),
            command: "exit 3".into(),
            restart: Restart::OnFailure,
            optional_binary: None,
        });
        sup.add(ServiceSpec {
            name: "absent".into(),
            command: "true".into(),
            restart: Restart::Always,
            optional_binary: Some("definitely-not-installed-edex".into()),
        });
        assert!(!sup.status().contains_key("absent"));
        let deadline = Instant::now() + Duration::from_secs(5);
        while sup.is_running("fails") && Instant::now() < deadline {
            std::thread::sleep(Duration::from_millis(20));
            sup.reap();
        }
        assert!(!sup.is_running("fails"));
        // Restarted after the 1 s back-off.
        std::thread::sleep(Duration::from_millis(1100));
        sup.tick();
        assert!(sup.is_running("fails"));
        sup.spawn("true", &[]).unwrap();
        sup.shutdown();
    }
}
