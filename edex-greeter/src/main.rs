//! eDEX greeter for greetd (runs as a fullscreen window under `cage`), or with its own login
//! backend on systems without PAM and greetd (RustOS, under labwc).

mod config;
mod greetd;
mod local;
mod screen;
mod sessions;
mod users;

use std::{
    path::PathBuf,
    time::{Duration, Instant},
};

use anyhow::{Context, Result};
use calloop::{
    channel,
    timer::{TimeoutAction, Timer},
    EventLoop,
};
use clap::Parser;
use config::{GreeterConfig, State};
use greetd::{Greetd, Step};
use platform::{button, KeyInput, Platform, PlatformEvent, SurfaceId};
use renderer::{GpuContext, SurfaceRenderer};
use sessions::Session;
use tracing::{error, info, warn};
use users::User;
use xkbcommon::xkb::keysyms as ks;

#[derive(Parser, Debug)]
#[command(name = "edex-greeter", version, about = "eDEX greeter for greetd")]
struct Cli {
    #[command(subcommand)]
    cmd: Option<Cmd>,
    /// Configuration file.
    #[arg(long, default_value = config::DEFAULT_PATH)]
    config: PathBuf,
    /// Run without greetd (renders the UI; Enter prints the session command).
    #[arg(long)]
    demo: bool,
    /// Login backend: greetd ($GREETD_SOCK), local (passwd/shadow, SHA-512 crypt; for systems
    /// without PAM such as RustOS), or auto (greetd when $GREETD_SOCK is set, local on
    /// RustOS, else demo).
    #[arg(long, default_value = "auto")]
    backend: String,
    /// Local backend: write the authenticated user's name here (else to stdout) and exit.
    #[arg(long, value_name = "FILE")]
    result: Option<PathBuf>,
    /// Exit after N seconds with a JSON report (CI).
    #[arg(long, value_name = "SECS")]
    smoke_test: Option<u64>,
}

#[derive(clap::Subcommand, Debug)]
enum Cmd {
    /// Become USER (uid, gid, HOME, USER, SHELL) and run COMMAND; for session scripts running
    /// as root after a local login.
    RunAs {
        user: String,
        #[arg(trailing_var_arg = true, required = true)]
        command: Vec<String>,
    },
}

/// Who checks passwords and starts the session.
pub enum Backend {
    Greetd(Greetd),
    Local(local::LocalLogin),
}

impl Backend {
    fn create_session(&mut self, user: &str) -> Result<Step> {
        match self {
            Backend::Greetd(g) => g.create_session(user),
            Backend::Local(l) => l.create_session(user),
        }
    }
    fn respond(&mut self, answer: Option<String>) -> Result<Step> {
        match self {
            Backend::Greetd(g) => g.respond(answer),
            Backend::Local(l) => l.respond(answer),
        }
    }
    fn start_session(&mut self, cmd: Vec<String>, env: Vec<String>) -> Result<()> {
        match self {
            Backend::Greetd(g) => g.start_session(cmd, env),
            Backend::Local(l) => l.start_session(cmd, env),
        }
    }
    fn cancel(&mut self) -> Result<()> {
        match self {
            Backend::Greetd(g) => g.cancel(),
            Backend::Local(l) => l.cancel(),
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Phase {
    PickUser,
    Prompt,
    Busy,
    Starting,
}

pub enum Event {
    Greetd(Box<(Backend, Result<Step>)>),
    Tick,
}

pub struct Greeter {
    pub cfg: GreeterConfig,
    pub theme: ::ui::Theme,
    pub metrics: ::ui::Metrics,
    pub users: Vec<User>,
    pub sessions: Vec<Session>,
    pub user_idx: usize,
    pub session_idx: usize,
    pub username_input: String,
    pub input: String,
    pub secret: bool,
    pub prompt: String,
    pub message: Option<(String, bool)>,
    pub phase: Phase,
    pub caps_lock: bool,
    pub hostname: String,
    pub clock: String,
    pub date: String,
    started: Instant,
    greetd: Option<Backend>,
    demo: bool,
    os: system::Os,
    tx: channel::Sender<Event>,
    quit: bool,
    exit_code: i32,
}

impl Greeter {
    pub fn pulse(&self) -> f32 {
        (self.started.elapsed().as_secs_f32() * 0.8).sin() * 0.5 + 0.5
    }

    fn current_user(&self) -> String {
        if self.cfg.show_users && !self.users.is_empty() {
            self.users
                .get(self.user_idx)
                .map(|u| u.name.clone())
                .unwrap_or_default()
        } else {
            self.username_input.trim().to_string()
        }
    }

    fn tick_clock(&mut self) {
        let now = chrono::Local::now();
        self.clock = now.format("%H:%M:%S").to_string();
        self.date = now.format("%A %d %B %Y").to_string();
    }

    fn set_message(&mut self, msg: impl Into<String>, error: bool) {
        self.message = Some((msg.into(), error));
    }

    /// Run a greetd call on a worker thread; the result comes back as `Event::Greetd`.
    fn call(&mut self, f: impl FnOnce(&mut Backend) -> Result<Step> + Send + 'static) {
        let Some(mut g) = self.greetd.take() else {
            if self.demo {
                self.demo_step();
            }
            return;
        };
        self.phase = Phase::Busy;
        let tx = self.tx.clone();
        std::thread::spawn(move || {
            let res = f(&mut g);
            let _ = tx.send(Event::Greetd(Box::new((g, res))));
        });
    }

    fn demo_step(&mut self) {
        match self.phase {
            Phase::PickUser => {
                self.phase = Phase::Prompt;
                self.prompt = "Password:".into();
                self.secret = true;
                self.input.clear();
                self.set_message("demo mode: any password is accepted", false);
            }
            Phase::Prompt => {
                let cmd = self.session_command();
                println!("{}", cmd.join(" "));
                self.set_message(format!("demo: would start `{}`", cmd.join(" ")), false);
                self.phase = Phase::PickUser;
                self.input.clear();
            }
            _ => {}
        }
    }

    fn session_command(&self) -> Vec<String> {
        self.sessions
            .get(self.session_idx)
            .map(|s| s.exec.clone())
            .unwrap_or_else(|| vec![self.cfg.fallback_command.clone()])
    }

    fn session_env(&self) -> Vec<String> {
        let id = self
            .sessions
            .get(self.session_idx)
            .map(|s| s.id.clone())
            .unwrap_or_else(|| "edex-de".into());
        let x11 = self.sessions.get(self.session_idx).is_some_and(|s| s.x11);
        let mut env = vec![
            format!("XDG_SESSION_TYPE={}", if x11 { "x11" } else { "wayland" }),
            format!("XDG_SESSION_DESKTOP={id}"),
        ];
        if id == "edex-de" {
            env.push("XDG_CURRENT_DESKTOP=eDEX-DE:Hyprland".into());
        }
        env
    }

    fn submit(&mut self) {
        match self.phase {
            Phase::PickUser => {
                let user = self.current_user();
                if user.is_empty() {
                    self.set_message("enter a user name", true);
                    return;
                }
                self.message = None;
                self.call(move |g| g.create_session(&user));
            }
            Phase::Prompt => {
                let answer = std::mem::take(&mut self.input);
                self.call(move |g| g.respond(Some(answer)));
            }
            _ => {}
        }
    }

    fn cancel(&mut self) {
        self.input.clear();
        self.phase = Phase::PickUser;
        self.message = None;
        self.call(|g| {
            g.cancel()?;
            Ok(Step::Failed("cancelled".into()))
        });
    }

    fn handle_step(&mut self, step: Result<Step>) {
        match step {
            Ok(Step::Prompt { message, secret }) => {
                self.phase = Phase::Prompt;
                self.prompt = message;
                self.secret = secret;
                self.input.clear();
            }
            Ok(Step::Info { message, error }) => {
                self.set_message(message, error);
                // Info messages need an empty answer to continue the conversation.
                self.call(|g| g.respond(None));
            }
            Ok(Step::Success) => {
                self.phase = Phase::Starting;
                self.set_message("starting session…", false);
                let cmd = self.session_command();
                let env = self.session_env();
                let state = State {
                    last_user: self.current_user(),
                    last_session: self
                        .sessions
                        .get(self.session_idx)
                        .map(|s| s.id.clone())
                        .unwrap_or_default(),
                };
                config::save_state(&self.cfg.state_file, &state);
                info!(cmd = ?cmd, "starting session");
                self.call(move |g| {
                    g.start_session(cmd, env)?;
                    Ok(Step::Success)
                });
                // The second Success (from start_session) ends the greeter.
                self.exit_code = 0;
            }
            Ok(Step::Failed(msg)) => {
                if msg != "cancelled" {
                    self.set_message(msg, true);
                }
                self.phase = Phase::PickUser;
                self.input.clear();
            }
            Err(e) => {
                error!("greetd: {e:#}");
                self.set_message(format!("greetd error: {e}"), true);
                self.phase = Phase::PickUser;
            }
        }
    }

    fn on_greetd(&mut self, g: Backend, step: Result<Step>) {
        self.greetd = Some(g);
        if self.phase == Phase::Starting {
            match step {
                Ok(_) => {
                    info!("session started; exiting greeter");
                    self.quit = true;
                }
                Err(e) => {
                    self.set_message(format!("cannot start session: {e}"), true);
                    self.phase = Phase::PickUser;
                }
            }
            return;
        }
        self.handle_step(step);
    }

    fn power(&mut self, action: system::LogindAction) {
        let res = match self.os {
            system::Os::RustOs => {
                system::rustos::power::logind(&system::RealRunner::default(), action)
            }
            system::Os::Linux => system::power::logind(action),
        };
        if let Err(e) = res {
            warn!("power action failed: {e:#}");
            self.set_message(format!("power action failed: {e}"), true);
        }
    }

    fn key(&mut self, key: &KeyInput) -> bool {
        if !key.pressed {
            return false;
        }
        self.caps_lock = key.modifiers.caps_lock;
        match key.keysym {
            ks::KEY_Return | ks::KEY_KP_Enter => self.submit(),
            ks::KEY_Escape => {
                if self.phase == Phase::Prompt {
                    self.cancel();
                } else {
                    self.input.clear();
                    self.username_input.clear();
                    self.message = None;
                }
            }
            ks::KEY_Up if self.phase == Phase::PickUser => {
                self.user_idx = self.user_idx.saturating_sub(1)
            }
            ks::KEY_Down if self.phase == Phase::PickUser => {
                self.user_idx = (self.user_idx + 1).min(self.users.len().saturating_sub(1))
            }
            ks::KEY_Tab | ks::KEY_ISO_Left_Tab if !self.sessions.is_empty() => {
                let n = self.sessions.len();
                self.session_idx = if key.modifiers.shift || key.keysym == ks::KEY_ISO_Left_Tab {
                    (self.session_idx + n - 1) % n
                } else {
                    (self.session_idx + 1) % n
                };
            }
            ks::KEY_F2 if self.cfg.power_buttons && self.can_suspend() => {
                self.power(system::LogindAction::Suspend)
            }
            ks::KEY_F3 if self.cfg.power_buttons => self.power(system::LogindAction::Reboot),
            ks::KEY_F4 if self.cfg.power_buttons => self.power(system::LogindAction::PowerOff),
            ks::KEY_BackSpace => {
                let target = self.active_input();
                if key.modifiers.ctrl {
                    target.clear();
                } else {
                    target.pop();
                }
            }
            _ => {
                if let Some(t) = key.text.as_deref() {
                    if !key.modifiers.ctrl
                        && !key.modifiers.alt
                        && !t.chars().any(|c| c.is_control())
                    {
                        self.active_input().push_str(t);
                    }
                }
            }
        }
        true
    }

    pub fn can_suspend(&self) -> bool {
        system::Capabilities::for_os(self.os).suspend
    }

    fn active_input(&mut self) -> &mut String {
        if self.phase == Phase::PickUser && !(self.cfg.show_users && !self.users.is_empty()) {
            &mut self.username_input
        } else {
            &mut self.input
        }
    }

    fn click(&mut self, id: u32) {
        match id {
            screen::HIT_SUBMIT => self.submit(),
            screen::HIT_SESSION_PREV => {
                let n = self.sessions.len().max(1);
                self.session_idx = (self.session_idx + n - 1) % n;
            }
            screen::HIT_SESSION_NEXT => {
                let n = self.sessions.len().max(1);
                self.session_idx = (self.session_idx + 1) % n;
            }
            screen::HIT_SUSPEND => self.power(system::LogindAction::Suspend),
            screen::HIT_REBOOT => self.power(system::LogindAction::Reboot),
            screen::HIT_POWEROFF => self.power(system::LogindAction::PowerOff),
            i if i >= screen::HIT_USER => {
                let idx = (i - screen::HIT_USER) as usize;
                if idx < self.users.len() && self.phase == Phase::PickUser {
                    if self.user_idx == idx {
                        self.submit();
                    } else {
                        self.user_idx = idx;
                    }
                }
            }
            _ => {}
        }
    }
}

fn main() {
    let cli = Cli::parse();
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| tracing_subscriber::EnvFilter::new("info")),
        )
        .with_writer(std::io::stderr)
        .init();
    match run(cli) {
        Ok(code) => std::process::exit(code),
        Err(e) => {
            error!("fatal: {e:#}");
            eprintln!("edex-greeter: {e:#}");
            std::process::exit(1);
        }
    }
}

fn run(cli: Cli) -> Result<i32> {
    if let Some(Cmd::RunAs { user, command }) = &cli.cmd {
        local::run_as(&local::Files::default(), user, command)?;
        return Ok(1);
    }
    let os = system::Os::detect();
    let mut cfg = config::load(&cli.config);
    if os == system::Os::RustOs && !cli.config.exists() {
        // RustOS has a single administrator account by default: root.
        cfg.min_uid = 0;
    }
    let share = settings_share_dir();
    let themes = ::ui::theme::load_themes(&[share.join("themes").as_path()]);
    let theme = themes
        .get(&cfg.theme)
        .cloned()
        .unwrap_or_else(::ui::theme::builtin_tron);
    let (mut event_loop, mut platform): (EventLoop<'static, Platform<Event>>, Platform<Event>) =
        Platform::new()?;
    let mut gpu = GpuContext::new(
        Some("JetBrainsMono Nerd Font".into()),
        Some(Box::new(platform.display_handle())),
    );
    let metrics = gpu.metrics(cfg.font_size, cfg.font_size);
    let (tx, rx) = channel::channel::<Event>();
    platform
        .loop_handle
        .insert_source(rx, |ev, _, p: &mut Platform<Event>| {
            if let channel::Event::Msg(e) = ev {
                p.push_app_event(e);
            }
        })
        .map_err(|e| anyhow::anyhow!("insert channel: {e}"))?;
    platform
        .loop_handle
        .insert_source(
            Timer::from_duration(Duration::from_millis(40)),
            |_, _, p: &mut Platform<Event>| {
                p.push_app_event(Event::Tick);
                TimeoutAction::ToDuration(Duration::from_millis(40))
            },
        )
        .map_err(|e| anyhow::anyhow!("insert timer: {e}"))?;

    let state = config::load_state(&cfg.state_file);
    let users = users::load(std::path::Path::new("/etc/passwd"), cfg.min_uid);
    let sessions = sessions::scan(&sessions::default_dirs());
    let user_idx = users
        .iter()
        .position(|u| u.name == state.last_user)
        .unwrap_or(0);
    let session_idx = sessions
        .iter()
        .position(|s| s.id == state.last_session)
        .or_else(|| sessions.iter().position(|s| s.id == cfg.default_session))
        .unwrap_or(0);
    let local = || {
        Backend::Local(local::LocalLogin::new(
            local::Files::default(),
            cli.result.clone(),
        ))
    };
    let greetd = match cli.backend.as_str() {
        _ if cli.demo => None,
        "local" => Some(local()),
        "greetd" => match Greetd::from_env() {
            Ok(g) => Some(Backend::Greetd(g)),
            Err(e) => {
                warn!("{e:#}; falling back to demo mode");
                None
            }
        },
        _ => match Greetd::from_env() {
            Ok(g) => Some(Backend::Greetd(g)),
            Err(_) if os == system::Os::RustOs => Some(local()),
            Err(e) => {
                warn!("{e:#}; falling back to demo mode");
                None
            }
        },
    };
    let demo = greetd.is_none();
    let mut g = Greeter {
        cfg,
        theme,
        metrics,
        users,
        sessions,
        user_idx,
        session_idx,
        username_input: state.last_user.clone(),
        input: String::new(),
        secret: true,
        prompt: String::new(),
        message: if demo {
            Some(("demo mode (no greetd socket)".into(), false))
        } else {
            None
        },
        phase: Phase::PickUser,
        caps_lock: false,
        hostname: std::fs::read_to_string("/etc/hostname")
            .map(|s| s.trim().to_string())
            .unwrap_or_else(|_| "edex".into()),
        clock: String::new(),
        date: String::new(),
        started: Instant::now(),
        greetd,
        demo,
        os,
        tx,
        quit: false,
        exit_code: 0,
    };
    g.tick_clock();

    let window: SurfaceId = platform
        .create_window("eDEX login", "edex-greeter", true)
        .context("creating the greeter window")?;
    let mut renderer: Option<SurfaceRenderer> = None;
    let mut hits = ::ui::HitMap::default();
    let mut dirty = true;
    let mut frames: u64 = 0;
    let mut pointer = (0.0f64, 0.0f64);
    let smoke = cli.smoke_test.map(Duration::from_secs);

    while !g.quit && !platform.should_exit() {
        platform.dispatch(&mut event_loop, Some(Duration::from_millis(50)))?;
        for ev in platform.drain_events() {
            match ev {
                PlatformEvent::Configure {
                    surface,
                    width,
                    height,
                    scale,
                } if surface == window => {
                    let (bw, bh) = platform.buffer_size(surface).unwrap_or((width, height));
                    match renderer.as_mut() {
                        Some(r) => r.resize(&gpu, bw, bh, scale as f32),
                        None => {
                            let handles = platform.raw_handles(surface)?;
                            renderer = Some(SurfaceRenderer::new(
                                &mut gpu,
                                handles,
                                bw,
                                bh,
                                scale as f32,
                            )?);
                        }
                    }
                    dirty = true;
                }
                PlatformEvent::ScaleChanged { surface, scale } if surface == window => {
                    if let Some(r) = renderer.as_mut() {
                        let (bw, bh) = platform.buffer_size(surface).unwrap_or(r.size());
                        r.resize(&gpu, bw, bh, scale as f32);
                    }
                    dirty = true;
                }
                PlatformEvent::Frame { .. } => dirty = true,
                PlatformEvent::Key { key, .. } => {
                    if g.key(&key) {
                        dirty = true;
                    }
                }
                PlatformEvent::ModifiersChanged { modifiers } => {
                    g.caps_lock = modifiers.caps_lock;
                    dirty = true;
                }
                PlatformEvent::PointerMotion { x, y, .. }
                | PlatformEvent::PointerEnter { x, y, .. } => pointer = (x, y),
                PlatformEvent::PointerButton {
                    button: b,
                    pressed: true,
                    x,
                    y,
                    ..
                } if b == button::LEFT => {
                    pointer = (x, y);
                    if let ::ui::HitTarget::OverlayItem(id) = hits.resolve(x as f32, y as f32) {
                        g.click(id);
                        dirty = true;
                    }
                }
                PlatformEvent::Closed { .. } => g.quit = true,
                PlatformEvent::App(Event::Greetd(boxed)) => {
                    let (gd, step) = *boxed;
                    g.on_greetd(gd, step);
                    dirty = true;
                }
                PlatformEvent::App(Event::Tick) => {
                    g.tick_clock();
                    dirty = true;
                    if let Some(s) = smoke {
                        if g.started.elapsed() >= s {
                            g.quit = true;
                        }
                    }
                }
                _ => {}
            }
        }
        let _ = pointer;
        if dirty && platform.is_configured(window) && !platform.frame_pending(window) {
            if let (Some(r), Some((w, h))) = (renderer.as_mut(), platform.logical_size(window)) {
                let rendered = screen::render(&g, w as f32, h as f32);
                hits = rendered.hits;
                if platform.request_frame(window) {
                    match r.render(&mut gpu, &rendered.scene) {
                        Ok(true) => {
                            frames += 1;
                            dirty = false;
                        }
                        Ok(false) => platform.commit(window),
                        Err(e) => {
                            error!("render: {e:#}");
                            platform.commit(window);
                        }
                    }
                }
            }
        }
    }
    if smoke.is_some() {
        println!(
            "{}",
            serde_json::json!({"frames": frames, "users": g.users.len(), "sessions": g.sessions.len(), "demo": g.demo})
        );
        return Ok(if frames >= 3 { 0 } else { 1 });
    }
    Ok(g.exit_code)
}

fn settings_share_dir() -> PathBuf {
    if let Some(d) = std::env::var_os("EDEX_SHARE_DIR").filter(|d| !d.is_empty()) {
        return PathBuf::from(d);
    }
    let usr = PathBuf::from("/usr/share/edex-de");
    let local = PathBuf::from("/usr/local/share/edex-de");
    if !usr.exists() && local.exists() {
        local
    } else {
        usr
    }
}
