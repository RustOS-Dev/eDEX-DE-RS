//! Control-socket requests and key-binding actions.

use std::{io::Write, path::PathBuf, process::Stdio};

use comp_proto::{Reply, Request, WindowId};
use smithay::{reexports::wayland_server::Resource, utils::Rectangle};
use tracing::{info, warn};

use crate::{
    binds::Action,
    ipc::Incoming,
    manage::from_rect,
    session,
    state::{Backend, CompMsg, EdexState, RunMode},
};

impl<B: Backend + 'static> EdexState<B> {
    pub fn handle_control(&mut self) {
        let incoming = match self.control.as_ref() {
            Some(c) => c.accept(),
            None => return,
        };
        for i in incoming {
            self.handle_request(i);
        }
    }

    fn handle_request(&mut self, incoming: Incoming) {
        let request = incoming.request.clone();
        let reply = match request {
            Request::Version => Reply::with_data(serde_json::json!({
                "version": env!("CARGO_PKG_VERSION"),
                "mode": match self.mode { RunMode::Greeter => "greeter", RunMode::Session => "session" },
            })),
            Request::State => match serde_json::to_value(self.snapshot()) {
                Ok(v) => Reply::with_data(v),
                Err(e) => Reply::err(e.to_string()),
            },
            Request::Subscribe => {
                if let Some(c) = self.control.as_mut() {
                    c.subscribe(incoming);
                }
                // The next flush diffs against an empty snapshot and sends the full state.
                return;
            }
            Request::Login {
                user,
                password,
                session,
            } => {
                self.login(incoming, user, password, session);
                return;
            }
            Request::PowerOff | Request::Reboot if self.mode == RunMode::Greeter => {
                let cmd = if request == Request::PowerOff {
                    "poweroff"
                } else {
                    "reboot"
                };
                match std::process::Command::new(cmd).spawn() {
                    Ok(_) => Reply::ok(),
                    Err(e) => Reply::err(format!("{cmd}: {e}")),
                }
            }
            Request::PowerOff | Request::Reboot => {
                Reply::err("power requests go through login1 in a session")
            }
            _ if self.mode == RunMode::Greeter => Reply::err("not available on the login screen"),
            Request::FocusWindow { id } => self.window_request(Some(id), |s, id| {
                s.wm.focus_window(id);
            }),
            Request::Close { id } => {
                self.close_window(id);
                Reply::ok()
            }
            Request::Kill { id } => {
                self.kill_window(id);
                Reply::ok()
            }
            Request::Minimize { id } => self.window_request(id, |s, id| s.wm.minimize(Some(id))),
            Request::Restore { id, workspace } => {
                self.window_request(Some(id), |s, id| s.wm.restore(id, workspace))
            }
            Request::ToggleMaximize { id } => {
                self.window_request(id, |s, id| s.wm.toggle_maximize(Some(id)))
            }
            Request::ToggleFullscreen { id } => {
                self.window_request(id, |s, id| s.wm.toggle_fullscreen(Some(id)))
            }
            Request::ToggleFloat { id } => self.window_request(id, |s, id| {
                let current =
                    s.wm.handle(id)
                        .and_then(|w| s.space.element_geometry(w))
                        .map(crate::manage::to_rect);
                s.wm.toggle_float(Some(id), current)
            }),
            Request::MoveToWorkspace {
                id,
                workspace,
                follow,
            } => self.window_request(id, |s, id| {
                s.wm.move_to_workspace(Some(id), workspace, follow)
            }),
            Request::FocusDirection { direction } => {
                self.wm.focus_direction(direction);
                self.mark_layout();
                Reply::ok()
            }
            Request::MoveDirection { direction } => {
                self.wm.move_direction(direction);
                self.mark_layout();
                Reply::ok()
            }
            Request::CycleFocus { reverse } => {
                self.wm.cycle_focus(reverse);
                self.mark_layout();
                Reply::ok()
            }
            Request::FocusWorkspace { workspace } => {
                self.wm.focus_workspace(workspace);
                self.mark_layout();
                Reply::ok()
            }
            Request::ToggleScratch => {
                self.wm.toggle_scratch();
                self.mark_layout();
                Reply::ok()
            }
            Request::SetAppArea {
                output,
                tiled,
                maximized,
            } => {
                if self.wm.set_app_area(&output, tiled, maximized) {
                    self.mark_layout();
                    Reply::ok()
                } else {
                    Reply::err(format!("no output named {output}"))
                }
            }
            Request::Exec { command, env } => match self.exec(&command, &env) {
                Ok(pid) => Reply::with_data(serde_json::json!({ "pid": pid })),
                Err(e) => Reply::err(e.to_string()),
            },
            Request::Lock => {
                self.lock_session();
                Reply::ok()
            }
            Request::Exit => {
                self.exit_session();
                Reply::ok()
            }
            Request::Binds => Reply::with_data(serde_json::json!({
                "binds": self.config.binds.iter().map(crate::binds::describe).collect::<Vec<_>>(),
            })),
            Request::Services => {
                let list: Vec<serde_json::Value> = self
                    .supervisor
                    .as_ref()
                    .map(|s| {
                        let mut v: Vec<_> = s.status().into_iter().collect();
                        v.sort();
                        v.into_iter()
                            .map(|(name, running)| serde_json::json!({"name": name, "running": running}))
                            .collect()
                    })
                    .unwrap_or_default();
                Reply::with_data(serde_json::json!({ "services": list }))
            }
            Request::RestartService { name } => match self.supervisor.as_mut() {
                Some(sup) => match sup.restart(&name) {
                    Ok(()) => Reply::ok(),
                    Err(e) => Reply::err(e),
                },
                None => Reply::err("no session"),
            },
            Request::Reload => {
                self.reload_config();
                Reply::with_data(serde_json::json!({ "bind_errors": self.config.bind_errors }))
            }
            Request::Dpms { on } => {
                B::set_dpms(self, on);
                Reply::ok()
            }
            Request::Screenshot {
                path,
                output,
                region,
            } => match B::screenshot(self, output.as_deref(), region.map(from_rect), &path) {
                Ok(()) => {
                    if let Some(sup) = self.supervisor.as_ref() {
                        let _ = session::chown(
                            std::path::Path::new(&path),
                            sup.user().uid,
                            sup.user().gid,
                        );
                    }
                    Reply::with_data(serde_json::json!({ "path": path }))
                }
                Err(e) => Reply::err(format!("{e:#}")),
            },
        };
        incoming.reply(&reply);
    }

    fn window_request(
        &mut self,
        id: Option<WindowId>,
        f: impl FnOnce(&mut Self, WindowId),
    ) -> Reply {
        match id.or(self.wm.focused()) {
            Some(id) if self.wm.handle(id).is_some() => {
                f(self, id);
                self.mark_layout();
                Reply::ok()
            }
            Some(id) => Reply::err(format!("no window {id}")),
            None => Reply::err("no focused window"),
        }
    }

    pub fn close_window(&mut self, id: Option<WindowId>) {
        let Some(window) = self.wm.close_target(id) else {
            return;
        };
        if let Some(toplevel) = window.0.toplevel() {
            toplevel.send_close();
        } else if let Some(x) = window.0.x11_surface() {
            let _ = x.close();
        }
    }

    pub fn kill_window(&mut self, id: Option<WindowId>) {
        let Some(window) = self.wm.close_target(id) else {
            return;
        };
        let pid = if let Some(x) = window.0.x11_surface() {
            x.pid().map(|p| p as i32)
        } else {
            window.wl_surface().and_then(|s| {
                self.display_handle
                    .get_client(s.id())
                    .ok()
                    .and_then(|c| c.get_credentials(&self.display_handle).ok())
                    .map(|c| c.pid)
            })
        };
        match pid {
            // SAFETY: plain kill(2).
            Some(pid) if pid > 1 => unsafe {
                libc::kill(pid, libc::SIGKILL);
            },
            _ => {
                if let Some(s) = window.wl_surface() {
                    if let Ok(client) = self.display_handle.get_client(s.id()) {
                        self.display_handle.backend_handle().kill_client(
                            client.id(),
                            smithay::reexports::wayland_server::backend::DisconnectReason::ConnectionClosed,
                        );
                    }
                }
            }
        }
    }

    /// Start a program in the session.
    pub fn exec(&mut self, command: &str, env: &[(String, String)]) -> std::io::Result<u32> {
        info!(command, "exec");
        match self.supervisor.as_mut() {
            Some(sup) => sup.spawn(command, env),
            None => Err(std::io::Error::other("no session")),
        }
    }

    /// Run a key-binding action.
    pub fn run_action(&mut self, action: Action) {
        if self.mode == RunMode::Greeter {
            if let Action::VtSwitch(vt) = action {
                B::switch_vt(self, vt);
            }
            return;
        }
        match action {
            Action::Shell(args) => self.shell_request(args),
            Action::Exec(cmd) => {
                if let Err(e) = self.exec(&cmd, &[]) {
                    warn!(cmd, "exec failed: {e}");
                }
            }
            Action::Close => self.close_window(None),
            Action::Kill => self.kill_window(None),
            Action::Minimize => self.wm.minimize(None),
            Action::ToggleMaximize => self.wm.toggle_maximize(None),
            Action::ToggleFullscreen => self.wm.toggle_fullscreen(None),
            Action::ToggleFloat => {
                let current = self
                    .wm
                    .focused()
                    .and_then(|id| self.wm.handle(id))
                    .and_then(|w| self.space.element_geometry(w))
                    .map(crate::manage::to_rect);
                self.wm.toggle_float(None, current);
            }
            Action::CenterFloat => {
                if let Some(id) = self.wm.focused().filter(|id| self.wm.is_floating(*id)) {
                    let current = self
                        .wm
                        .handle(id)
                        .and_then(|w| self.space.element_geometry(w))
                        .map(crate::manage::to_rect);
                    // Re-floating recentres it.
                    self.wm.toggle_float(Some(id), None);
                    self.wm.set_float_rect(id, comp_proto::Rect::default());
                    self.wm.toggle_float(Some(id), current);
                }
            }
            Action::Focus(d) => self.wm.focus_direction(d),
            Action::Move(d) => self.wm.move_direction(d),
            Action::Cycle { reverse } => self.wm.cycle_focus(reverse),
            Action::Workspace(t) => self.wm.focus_workspace(t),
            Action::MoveToWorkspace { target, follow } => {
                self.wm.move_to_workspace(None, target, follow)
            }
            Action::ToggleScratch => self.wm.toggle_scratch(),
            Action::Lock => self.lock_session(),
            Action::Exit => self.exit_session(),
            Action::Reload => self.reload_config(),
            Action::Screenshot { region } => self.screenshot_action(region),
            Action::VtSwitch(vt) => B::switch_vt(self, vt),
            Action::None => {}
        }
        self.mark_layout();
    }

    /// Forward an `edex-de ipc …` request to the shell without blocking the compositor.
    fn shell_request(&mut self, args: Vec<String>) {
        let request = match ::ipc::proto::parse_args(&args) {
            Ok(r) => r,
            Err(e) => {
                warn!("bad shell binding {args:?}: {e}");
                return;
            }
        };
        let path = self.runtime_dir.join("edex-de").join("ipc.sock");
        std::thread::spawn(move || {
            if let Err(e) = ::ipc::send(&path, &request) {
                warn!("shell request failed: {e:#}");
            }
        });
    }

    fn screenshot_action(&mut self, region: bool) {
        let dir = self
            .supervisor
            .as_ref()
            .map(|s| s.user().home.join("Pictures/Screenshots"))
            .unwrap_or_else(|| PathBuf::from("/tmp"));
        if region {
            // slurp picks the region; grim reads it through wlr-screencopy when installed,
            // otherwise ask edex-comp for the region through the control socket.
            let cmd = "g=$(slurp) || exit 0; \
                 if command -v grim >/dev/null; then grim -g \"$g\" - | wl-copy; \
                 else f=$(mktemp /tmp/shot-XXXXXX.png); edex-comp screenshot --region \"$g\" \"$f\" && wl-copy < \"$f\"; rm -f \"$f\"; fi; \
                 edex-de ipc notify Screenshot copied to clipboard";
            let _ = self.exec(cmd, &[]);
            return;
        }
        let _ = std::fs::create_dir_all(&dir);
        if let Some(sup) = self.supervisor.as_ref() {
            let _ = session::chown(&dir, sup.user().uid, sup.user().gid);
        }
        let name = format!(
            "{}.png",
            chrono_like_timestamp().unwrap_or_else(|| "screenshot".into())
        );
        let path = dir.join(name);
        let output = self.wm.focused_output_name().map(str::to_string);
        match B::screenshot(self, output.as_deref(), None, &path.to_string_lossy()) {
            Ok(()) => {
                if let Some(sup) = self.supervisor.as_ref() {
                    let _ = session::chown(&path, sup.user().uid, sup.user().gid);
                }
                self.shell_request(vec![
                    "notify".into(),
                    "Screenshot saved".into(),
                    path.display().to_string(),
                ]);
            }
            Err(e) => warn!("screenshot failed: {e:#}"),
        }
    }

    /// Greeter login or lock-screen unlock: check the password with `edex-auth` off the event
    /// loop and reply when it answers.
    fn login(
        &mut self,
        incoming: Incoming,
        user: String,
        password: String,
        session_name: Option<String>,
    ) {
        if self.mode == RunMode::Session {
            // Only the session's own user can unlock it.
            let own = self
                .supervisor
                .as_ref()
                .map(|s| s.user().name.clone())
                .unwrap_or_default();
            if user != own {
                incoming.reply(&Reply::err("this session belongs to another user"));
                return;
            }
        }
        let tx = self.msg_tx.clone();
        let mode = self.mode;
        std::thread::spawn(move || {
            let result = check_password(&user, &password);
            match result {
                Ok(()) => {
                    incoming.reply(&Reply::ok());
                    // Unlocking is the lock client's own ext-session-lock request.
                    if mode == RunMode::Greeter {
                        let _ = tx.send(CompMsg::LoginOk {
                            user,
                            session: session_name,
                        });
                    }
                }
                Err(e) => incoming.reply(&Reply::err(e)),
            }
        });
    }
}

/// Ask `edex-auth check USER` (password on stdin). Exit 0 means the password is right.
pub fn check_password(user: &str, password: &str) -> Result<(), String> {
    let helper = session::which("edex-auth").ok_or("edex-auth is not installed")?;
    let mut child = std::process::Command::new(helper)
        .args(["check", user])
        .stdin(Stdio::piped())
        .stdout(Stdio::null())
        .stderr(Stdio::piped())
        .spawn()
        .map_err(|e| format!("edex-auth: {e}"))?;
    if let Some(mut stdin) = child.stdin.take() {
        let _ = stdin.write_all(password.as_bytes());
        let _ = stdin.write_all(b"\n");
    }
    let out = child
        .wait_with_output()
        .map_err(|e| format!("edex-auth: {e}"))?;
    if out.status.success() {
        Ok(())
    } else {
        let msg = String::from_utf8_lossy(&out.stderr).trim().to_string();
        Err(if msg.is_empty() {
            "authentication failed".into()
        } else {
            msg
        })
    }
}

/// `YYYYmmdd-HHMMSS` in local time, from `date` (no time-zone database in this process).
fn chrono_like_timestamp() -> Option<String> {
    let out = std::process::Command::new("date")
        .arg("+%Y%m%d-%H%M%S")
        .output()
        .ok()?;
    out.status
        .success()
        .then(|| String::from_utf8_lossy(&out.stdout).trim().to_string())
}

/// A rectangle from `slurp`'s `X,Y WxH` format.
pub fn parse_slurp(s: &str) -> Option<Rectangle<i32, smithay::utils::Logical>> {
    let (pos, size) = s.trim().split_once(' ')?;
    let (x, y) = pos.split_once(',')?;
    let (w, h) = size.split_once('x')?;
    Some(Rectangle::new(
        (x.parse().ok()?, y.parse().ok()?).into(),
        (w.parse().ok()?, h.parse().ok()?).into(),
    ))
}

#[cfg(test)]
mod tests {
    #[test]
    fn parses_slurp_geometry() {
        let r = super::parse_slurp("10,20 300x200\n").unwrap();
        assert_eq!((r.loc.x, r.loc.y, r.size.w, r.size.h), (10, 20, 300, 200));
        assert!(super::parse_slurp("garbage").is_none());
    }
}
