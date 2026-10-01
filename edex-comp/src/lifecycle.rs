//! Session lifecycle: the greeter, logging in, locking, idle, configuration reloads and exit.

use std::{
    collections::HashSet,
    sync::atomic::Ordering,
    time::{Duration, Instant},
};

use smithay::{
    delegate_foreign_toplevel_list, delegate_idle_inhibit, delegate_idle_notify,
    delegate_session_lock, delegate_xdg_system_bell,
    input::keyboard::XkbConfig,
    output::Output,
    reexports::wayland_server::protocol::{wl_output::WlOutput, wl_surface::WlSurface},
    utils::SERIAL_COUNTER,
    wayland::{
        compositor::with_states,
        foreign_toplevel_list::{ForeignToplevelListHandler, ForeignToplevelListState},
        idle_inhibit::IdleInhibitHandler,
        idle_notify::{IdleNotifierHandler, IdleNotifierState},
        session_lock::{LockSurface, SessionLockHandler, SessionLockManagerState, SessionLocker},
        xdg_system_bell::XdgSystemBellHandler,
    },
};
use tracing::{error, info, warn};

use crate::{
    config::CompConfig,
    focus::KeyboardFocusTarget,
    ipc::ControlServer,
    session::{self, Restart, ServiceSpec, Supervisor, User},
    state::{Backend, EdexState, RunMode},
};

/// Session-lock state (ext-session-lock-v1).
#[derive(Debug, Default)]
pub struct LockState {
    pub locked: bool,
    /// Lock surfaces by output name.
    pub surfaces: Vec<(String, LockSurface)>,
    /// The lock client we started, while it runs.
    pub client_pid: Option<u32>,
    pub requested_at: Option<Instant>,
}

/// Inactivity tracking for idle lock and DPMS.
#[derive(Debug)]
pub struct IdleState {
    pub last_activity: Instant,
    pub inhibitors: HashSet<WlSurface>,
    pub dpms_off: bool,
}

impl Default for IdleState {
    fn default() -> Self {
        Self {
            last_activity: Instant::now(),
            inhibitors: HashSet::new(),
            dpms_off: false,
        }
    }
}

impl<B: Backend + 'static> EdexState<B> {
    /// Greeter mode: run `edex-greeter` until someone logs in.
    pub fn start_greeter(&mut self) {
        let user = session::current_user();
        let env = session::session_env(
            &user,
            &self.runtime_dir,
            self.socket_name.as_deref(),
            None,
            self.control
                .as_ref()
                .map(|c| c.path().to_path_buf())
                .unwrap_or_default()
                .as_path(),
        );
        let mut sup = Supervisor::new(user, env);
        sup.add(ServiceSpec {
            name: "greeter".into(),
            command: "exec edex-greeter".into(),
            restart: Restart::Always,
            optional_binary: None,
        });
        self.supervisor = Some(sup);
    }

    /// No greeter: the session belongs to whoever started edex-comp (from a tty, or nested
    /// for development).
    pub fn start_direct_session(&mut self) {
        let user = session::current_user();
        self.mode = RunMode::Session;
        self.start_xwayland();
        let env = session::session_env(
            &user,
            &self.runtime_dir,
            self.socket_name.as_deref(),
            self.xdisplay,
            self.control
                .as_ref()
                .map(|c| c.path().to_path_buf())
                .unwrap_or_default()
                .as_path(),
        );
        let mut sup = Supervisor::new(user, env);
        self.add_session_programs(&mut sup);
        self.supervisor = Some(sup);
        self.mark_layout();
    }

    fn add_session_programs(&self, sup: &mut Supervisor) {
        match &self.autostart {
            Some(commands) => {
                for (i, cmd) in commands.iter().enumerate() {
                    sup.add(ServiceSpec {
                        name: format!("run-{i}"),
                        command: cmd.clone(),
                        restart: Restart::Never,
                        optional_binary: None,
                    });
                }
            }
            None => {
                for spec in session::default_services() {
                    sup.add(spec);
                }
            }
        }
    }

    /// Start the user's session: their sockets, config, Xwayland and the session programs.
    pub fn start_session(&mut self, user: User) {
        info!(user = user.name, "starting session");
        if let Some(mut greeter) = self.supervisor.take() {
            greeter.shutdown();
        }
        let runtime_dir = match session::ensure_runtime_dir(&user) {
            Ok(d) => d,
            Err(e) => {
                error!("cannot create the runtime directory for {}: {e}", user.name);
                return;
            }
        };
        // A Wayland socket and a control socket the user can reach.
        if self.mode == RunMode::Greeter {
            match self.listen_wayland_in(&runtime_dir) {
                Ok(name) => {
                    let _ = session::chown(&runtime_dir.join(&name), user.uid, user.gid);
                    let _ = session::chown(
                        &runtime_dir.join(format!("{name}.lock")),
                        user.uid,
                        user.gid,
                    );
                    self.socket_name = Some(name);
                }
                Err(e) => error!("cannot open a Wayland socket for the session: {e:#}"),
            }
            let path = runtime_dir.join("edex-comp.sock");
            match ControlServer::bind(&path) {
                Ok(server) => {
                    let _ = session::chown(&path, user.uid, user.gid);
                    self.control = Some(server);
                }
                Err(e) => error!("cannot open the control socket: {e:#}"),
            }
        }
        self.runtime_dir = runtime_dir.clone();
        self.mode = RunMode::Session;
        self.config = CompConfig::load(&user.config_path());
        self.config_watcher = settings::watch(&user.config_path()).ok();
        self.apply_config();
        self.start_xwayland();

        let env = session::session_env(
            &user,
            &runtime_dir,
            self.socket_name.as_deref(),
            self.xdisplay,
            self.control
                .as_ref()
                .map(|c| c.path().to_path_buf())
                .unwrap_or_default()
                .as_path(),
        );
        let mut sup = Supervisor::new(user, env);
        self.add_session_programs(&mut sup);
        self.supervisor = Some(sup);
        self.mark_layout();
    }

    /// Ask the user's session to end; edex-comp exits and the service manager brings the
    /// greeter back.
    pub fn exit_session(&mut self) {
        info!("ending the session");
        if let Some(sup) = self.supervisor.as_mut() {
            sup.shutdown();
        }
        self.running.store(false, Ordering::SeqCst);
    }

    /// Start the lock screen (it locks through ext-session-lock).
    pub fn lock_session(&mut self) {
        if self.mode != RunMode::Session || self.lock.locked || self.lock_client_running() {
            return;
        }
        let Some(sup) = self.supervisor.as_mut() else {
            return;
        };
        match sup.spawn("exec edex-greeter --lock", &[]) {
            Ok(pid) => {
                self.lock.client_pid = Some(pid);
                self.lock.requested_at = Some(Instant::now());
            }
            Err(e) => error!("cannot start the lock screen: {e}"),
        }
    }

    fn lock_client_running(&self) -> bool {
        // SAFETY: kill with signal 0 only checks that the process exists.
        self.lock
            .client_pid
            .is_some_and(|pid| unsafe { libc::kill(pid as i32, 0) } == 0)
    }

    pub fn reload_config(&mut self) {
        let path = self.config.source.clone();
        self.config = CompConfig::load(&path);
        for e in &self.config.bind_errors {
            warn!("binding ignored: {e}");
        }
        self.apply_config();
        if let Some(c) = self.control.as_mut() {
            c.broadcast(&[comp_proto::Event::ConfigReloaded]);
        }
    }

    /// Apply input, layout, output and night-light settings.
    pub fn apply_config(&mut self) {
        self.wm.config = self.config.wm;
        let x = self.config.xkb.clone();
        let keyboard = self.seat.get_keyboard().unwrap();
        let xkb = XkbConfig {
            rules: "evdev",
            model: "pc105",
            layout: &x.layout,
            variant: &x.variant,
            options: (!x.options.is_empty()).then(|| x.options.clone()),
        };
        if let Err(e) = keyboard.set_xkb_config(self, xkb) {
            warn!("keyboard layout {:?} rejected: {e:?}", x.layout);
        }
        keyboard.change_repeat_info(x.repeat_rate, x.repeat_delay);
        self.update_keyboard_layout_name();
        B::apply_input_config(self);
        B::apply_output_config(self);
        B::set_gamma(self, self.config.night_light);
        self.mark_layout();
    }

    pub fn update_keyboard_layout_name(&mut self) {
        let keyboard = self.seat.get_keyboard().unwrap();
        let name = keyboard.with_xkb_state(self, |ctx| {
            let xkb = ctx.xkb().lock().unwrap();
            let layout = xkb.active_layout();
            // SAFETY: the keymap outlives this call.
            unsafe { xkb.keymap() }
                .layout_get_name(layout.0)
                .to_string()
        });
        self.keyboard_layout = name;
    }

    /// Once a second: reap and restart session programs, watch the config, idle actions.
    pub fn tick(&mut self) {
        if let Some(sup) = self.supervisor.as_mut() {
            sup.reap();
            sup.tick();
        }
        if self.config_watcher.as_ref().is_some_and(|w| w.changed()) {
            self.reload_config();
        }
        // A crashed lock screen leaves the session locked: start another.
        if self.lock.locked && !self.lock_client_running() {
            self.lock.client_pid = None;
            if let Some(sup) = self.supervisor.as_mut() {
                if let Ok(pid) = sup.spawn("exec edex-greeter --lock", &[]) {
                    self.lock.client_pid = Some(pid);
                }
            }
        }
        if self.mode != RunMode::Session {
            return;
        }
        let idle = self.idle.last_activity.elapsed();
        let inhibited = !self.idle.inhibitors.is_empty();
        let cfg = self.config.idle;
        if !inhibited {
            if cfg.lock_after > 0 && idle >= Duration::from_secs(cfg.lock_after as u64) {
                self.lock_session();
            }
            if cfg.dpms_after > 0
                && !self.idle.dpms_off
                && idle >= Duration::from_secs(cfg.dpms_after as u64)
            {
                self.idle.dpms_off = true;
                B::set_dpms(self, false);
            }
        }
    }

    /// Any input event.
    pub fn on_activity(&mut self) {
        self.idle.last_activity = Instant::now();
        let seat = self.seat.clone();
        self.idle_notifier_state.notify_activity(&seat);
        if self.idle.dpms_off {
            self.idle.dpms_off = false;
            B::set_dpms(self, true);
        }
    }

    /// The lid closed or opened (libinput switch).
    pub fn on_lid(&mut self, closed: bool) {
        use crate::config::LidAction;
        if !closed {
            B::set_dpms(self, true);
            return;
        }
        match self.config.lid {
            LidAction::Ignore => {}
            LidAction::Lock => self.lock_session(),
            LidAction::Suspend => {
                if self.config.idle.lock_on_lid {
                    self.lock_session();
                }
                B::set_dpms(self, false);
            }
            LidAction::PowerOff => {
                self.exit_session();
                let _ = std::process::Command::new("poweroff").spawn();
            }
        }
    }

    pub fn lock_surface_for(&self, output: &Output) -> Option<WlSurface> {
        self.lock
            .surfaces
            .iter()
            .find(|(name, s)| *name == output.name() && s.alive())
            .map(|(_, s)| s.wl_surface().clone())
    }

    fn focus_lock_surface(&mut self) {
        let name = self
            .wm
            .focused_output_name()
            .map(str::to_string)
            .or_else(|| self.lock.surfaces.first().map(|(n, _)| n.clone()));
        let surface = self
            .lock
            .surfaces
            .iter()
            .find(|(n, s)| Some(n) == name.as_ref() && s.alive())
            .or_else(|| self.lock.surfaces.iter().find(|(_, s)| s.alive()))
            .map(|(_, s)| s.wl_surface().clone());
        let keyboard = self.seat.get_keyboard().unwrap();
        keyboard.set_focus(
            self,
            surface.map(KeyboardFocusTarget::LockSurface),
            SERIAL_COUNTER.next_serial(),
        );
    }
}

impl<B: Backend + 'static> SessionLockHandler for EdexState<B> {
    fn lock_state(&mut self) -> &mut SessionLockManagerState {
        &mut self.session_lock_state
    }

    fn lock(&mut self, confirmation: SessionLocker) {
        info!("session locked");
        self.lock.locked = true;
        self.lock.requested_at = None;
        confirmation.lock();
        let keyboard = self.seat.get_keyboard().unwrap();
        keyboard.set_focus(self, None, SERIAL_COUNTER.next_serial());
        for output in self.space.outputs().cloned().collect::<Vec<_>>() {
            B::reset_buffers_for(self, &output);
        }
    }

    fn unlock(&mut self) {
        info!("session unlocked");
        self.lock.locked = false;
        self.lock.surfaces.clear();
        self.lock.client_pid = None;
        self.on_activity();
        let keyboard = self.seat.get_keyboard().unwrap();
        keyboard.set_focus(self, None, SERIAL_COUNTER.next_serial());
        self.mark_layout();
    }

    fn new_surface(&mut self, surface: LockSurface, output: WlOutput) {
        let Some(output) = Output::from_resource(&output) else {
            return;
        };
        let size = output
            .current_mode()
            .map(|m| {
                output
                    .current_transform()
                    .transform_size(m.size)
                    .to_f64()
                    .to_logical(output.current_scale().fractional_scale())
                    .to_i32_round::<i32>()
            })
            .unwrap_or_default();
        surface.with_pending_state(|s| {
            s.size = Some((size.w.max(1) as u32, size.h.max(1) as u32).into());
        });
        surface.send_configure();
        self.lock
            .surfaces
            .retain(|(name, s)| *name != output.name() && s.alive());
        self.lock.surfaces.push((output.name(), surface));
        self.focus_lock_surface();
    }
}
delegate_session_lock!(@<B: Backend + 'static> EdexState<B>);

impl<B: Backend + 'static> IdleNotifierHandler for EdexState<B> {
    fn idle_notifier_state(&mut self) -> &mut IdleNotifierState<Self> {
        &mut self.idle_notifier_state
    }
}
delegate_idle_notify!(@<B: Backend + 'static> EdexState<B>);

impl<B: Backend + 'static> IdleInhibitHandler for EdexState<B> {
    fn inhibit(&mut self, surface: WlSurface) {
        self.idle.inhibitors.insert(surface);
        self.idle_notifier_state.set_is_inhibited(true);
    }

    fn uninhibit(&mut self, surface: WlSurface) {
        self.idle.inhibitors.remove(&surface);
        self.idle.inhibitors.retain(|s| {
            with_states(s, |_| true) && smithay::reexports::wayland_server::Resource::is_alive(s)
        });
        let inhibited = !self.idle.inhibitors.is_empty();
        self.idle_notifier_state.set_is_inhibited(inhibited);
    }
}
delegate_idle_inhibit!(@<B: Backend + 'static> EdexState<B>);

impl<B: Backend + 'static> XdgSystemBellHandler for EdexState<B> {
    fn ring(&mut self, _surface: Option<WlSurface>) {
        if let Some(c) = self.control.as_mut() {
            c.broadcast(&[comp_proto::Event::Bell]);
        }
    }
}
delegate_xdg_system_bell!(@<B: Backend + 'static> EdexState<B>);

impl<B: Backend + 'static> ForeignToplevelListHandler for EdexState<B> {
    fn foreign_toplevel_list_state(&mut self) -> &mut ForeignToplevelListState {
        &mut self.foreign_toplevel_state
    }
}
delegate_foreign_toplevel_list!(@<B: Backend + 'static> EdexState<B>);
