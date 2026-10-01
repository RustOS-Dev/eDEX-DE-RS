//! eDEX greeter for RustOS. On the login screen it is a fullscreen window of edex-comp running
//! in greeter mode; with `--lock` it is the session's lock screen (ext-session-lock). Either way
//! the password goes to edex-comp, which checks it with edex-auth.

mod config;
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
use comp_proto::Request;
use config::{GreeterConfig, State};
use platform::{button, KeyInput, Platform, PlatformEvent, SurfaceId};
use renderer::{GpuContext, SurfaceRenderer};
use sessions::Session;
use tracing::{error, info, warn};
use users::User;
use xkbcommon::xkb::keysyms as ks;

#[derive(Parser, Debug)]
#[command(name = "edex-greeter", version, about = "eDEX login and lock screen")]
struct Cli {
    /// Configuration file.
    #[arg(long, default_value = config::DEFAULT_PATH)]
    config: PathBuf,
    /// Be the lock screen of the running session.
    #[arg(long)]
    lock: bool,
    /// Run without edex-comp (renders the UI; any password is accepted).
    #[arg(long)]
    demo: bool,
    /// Exit after N seconds with a JSON report (CI).
    #[arg(long, value_name = "SECS")]
    smoke_test: Option<u64>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Phase {
    PickUser,
    Prompt,
    Busy,
    Starting,
}

pub enum Event {
    /// edex-comp's answer to a login (or unlock) request.
    Login(std::result::Result<(), String>),
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
    demo: bool,
    /// Lock screen of a running session rather than the login screen.
    pub lock: bool,
    /// Set when a lock-screen password was accepted: unlock and exit.
    unlocked: bool,
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

    /// Ask edex-comp to check the password on a worker thread; the answer comes back as
    /// `Event::Login`.
    fn login(&mut self) {
        let user = self.current_user();
        let password = std::mem::take(&mut self.input);
        if self.demo {
            self.set_message(format!("demo: would log in {user}"), false);
            self.phase = Phase::Prompt;
            return;
        }
        let session = self.sessions.get(self.session_idx).map(|s| s.id.clone());
        self.phase = Phase::Busy;
        self.message = None;
        let tx = self.tx.clone();
        std::thread::spawn(move || {
            let res = comp_proto::call(&Request::Login {
                user,
                password,
                session,
            })
            .map(|_| ())
            .map_err(|e| e.to_string());
            let _ = tx.send(Event::Login(res));
        });
    }

    fn submit(&mut self) {
        match self.phase {
            Phase::PickUser => {
                if self.current_user().is_empty() {
                    self.set_message("enter a user name", true);
                    return;
                }
                self.message = None;
                self.phase = Phase::Prompt;
                self.prompt = "Password:".into();
                self.secret = true;
                self.input.clear();
            }
            Phase::Prompt => self.login(),
            _ => {}
        }
    }

    fn cancel(&mut self) {
        self.input.clear();
        self.message = None;
        if !self.lock {
            self.phase = Phase::PickUser;
        }
    }

    fn on_login(&mut self, res: std::result::Result<(), String>) {
        match res {
            Ok(()) if self.lock => {
                info!("password accepted; unlocking");
                self.unlocked = true;
            }
            Ok(()) => {
                self.phase = Phase::Starting;
                self.set_message("starting session…", false);
                let state = State {
                    last_user: self.current_user(),
                    last_session: self
                        .sessions
                        .get(self.session_idx)
                        .map(|s| s.id.clone())
                        .unwrap_or_default(),
                };
                config::save_state(&self.cfg.state_file, &state);
                // edex-comp now starts the session and ends this greeter.
            }
            Err(msg) => {
                self.set_message(msg, true);
                self.phase = Phase::Prompt;
                self.input.clear();
            }
        }
    }

    /// Reboot or power off from the login screen (edex-comp runs them; there is no suspend
    /// on RustOS).
    fn power(&mut self, request: Request) {
        if self.lock || self.demo {
            return;
        }
        if let Err(e) = comp_proto::call(&request) {
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
            ks::KEY_F3 if self.cfg.power_buttons => self.power(Request::Reboot),
            ks::KEY_F4 if self.cfg.power_buttons => self.power(Request::PowerOff),
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
            screen::HIT_REBOOT => self.power(Request::Reboot),
            screen::HIT_POWEROFF => self.power(Request::PowerOff),
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
    let mut cfg = config::load(&cli.config);
    if cli.lock {
        // The lock screen belongs to one user and cannot power the machine off.
        cfg.show_users = false;
        cfg.power_buttons = false;
    }
    let share = settings_share_dir();
    let themes = ::ui::theme::load_themes(&[share.join("themes").as_path()]);
    let theme = themes
        .get(&cfg.theme)
        .cloned()
        .unwrap_or_else(::ui::theme::builtin_tron);
    let mut gpu = GpuContext::new(Some("JetBrainsMono Nerd Font".into()));
    let metrics = gpu.metrics(cfg.font_size, cfg.font_size);

    let (mut event_loop, mut platform): (EventLoop<'static, Platform<Event>>, Platform<Event>) =
        Platform::new()?;
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
    let users = users::load(cfg.min_uid);
    let sessions = sessions::edex_sessions();
    let user_idx = users
        .iter()
        .position(|u| u.name == state.last_user)
        .unwrap_or(0);
    let session_idx = sessions
        .iter()
        .position(|s| s.id == state.last_session)
        .or_else(|| sessions.iter().position(|s| s.id == cfg.default_session))
        .unwrap_or(0);
    let demo = cli.demo || comp_proto::socket_path().is_none_or(|p| !p.exists());
    if demo && !cli.demo {
        warn!("edex-comp is not running; demo mode");
    }
    let lock_user = std::env::var("USER").unwrap_or_else(|_| state.last_user.clone());
    let mut g = Greeter {
        cfg,
        theme,
        metrics,
        users,
        sessions,
        user_idx,
        session_idx,
        username_input: if cli.lock {
            lock_user.clone()
        } else {
            state.last_user.clone()
        },
        input: String::new(),
        secret: true,
        prompt: if cli.lock {
            format!("Password for {lock_user}:")
        } else {
            String::new()
        },
        message: if demo {
            Some(("demo mode (edex-comp is not running)".into(), false))
        } else {
            None
        },
        phase: if cli.lock {
            Phase::Prompt
        } else {
            Phase::PickUser
        },
        caps_lock: false,
        hostname: std::fs::read_to_string("/storage/etc/hostname")
            .or_else(|_| std::fs::read_to_string("/etc/hostname"))
            .map(|s| s.trim().to_string())
            .unwrap_or_else(|_| "edex".into()),
        clock: String::new(),
        date: String::new(),
        started: Instant::now(),
        demo,
        lock: cli.lock,
        unlocked: false,
        tx,
        quit: false,
        exit_code: 0,
    };
    g.tick_clock();

    let surfaces: Vec<SurfaceId> = if cli.lock {
        platform.lock_session().context("locking the session")?
    } else {
        vec![platform
            .create_window("eDEX login", "edex-greeter", true)
            .context("creating the greeter window")?]
    };
    let mut renderers: std::collections::HashMap<SurfaceId, SurfaceRenderer> =
        std::collections::HashMap::new();
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
                } if surfaces.contains(&surface) => {
                    let (bw, bh) = platform.buffer_size(surface).unwrap_or((width, height));
                    match renderers.get_mut(&surface) {
                        Some(r) => r.resize(&gpu, bw, bh, scale as f32),
                        None => {
                            let handles = platform.raw_handles(surface)?;
                            renderers.insert(
                                surface,
                                SurfaceRenderer::new(&mut gpu, handles, bw, bh, scale as f32)?,
                            );
                        }
                    }
                    dirty = true;
                }
                PlatformEvent::ScaleChanged { surface, scale } if surfaces.contains(&surface) => {
                    if let Some(r) = renderers.get_mut(&surface) {
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
                PlatformEvent::App(Event::Login(res)) => {
                    g.on_login(res);
                    if g.unlocked {
                        platform.unlock_session();
                        g.quit = true;
                    }
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
        if dirty {
            let mut drawn = 0;
            for (i, surface) in surfaces.iter().enumerate() {
                if !platform.is_configured(*surface) || platform.frame_pending(*surface) {
                    continue;
                }
                let (Some(r), Some((w, h))) =
                    (renderers.get_mut(surface), platform.logical_size(*surface))
                else {
                    continue;
                };
                let rendered = screen::render(&g, w as f32, h as f32);
                // Clicks are resolved against the first surface's layout.
                if i == 0 {
                    hits = rendered.hits;
                }
                if platform.request_frame(*surface) {
                    match r.render(&mut gpu, &rendered.scene) {
                        Ok(true) => drawn += 1,
                        Ok(false) => platform.commit(*surface),
                        Err(e) => {
                            error!("render: {e:#}");
                            platform.commit(*surface);
                        }
                    }
                }
            }
            if drawn > 0 {
                frames += 1;
                dirty = false;
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
    std::env::var_os("EDEX_SHARE_DIR")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from("/usr/share/edex-de"))
}
