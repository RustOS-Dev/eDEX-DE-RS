//! System backends for the settings and privacy panels. Blocking work runs on a worker
//! thread; results come back through a caller-provided sink.

pub mod about;
pub mod audio;
pub mod bluetooth;
pub mod brightness;
pub mod display;
pub mod fprint;
pub mod input;
pub mod network;
pub mod power;
pub mod privacy;
pub mod runner;
pub mod services;
pub mod users;

use std::{
    sync::{mpsc, Arc},
    thread,
};

use comp::CompSocket;
pub use power::LogindAction;
pub use runner::{CommandRunner, FakeRunner, RealRunner};
pub use services::UnitAction;

/// Work items for the backend thread.
#[derive(Clone, Debug)]
pub enum SysRequest {
    AudioQuery,
    AudioSetVolume(u32),
    AudioAdjustVolume(i32),
    AudioToggleMute,
    AudioToggleMicMute,
    AudioSetDefault(u32),
    BrightnessQuery,
    BrightnessAdjust(i32),
    BrightnessSet(u32),
    NetworkQuery {
        rescan: bool,
    },
    WifiConnect {
        ssid: String,
        password: Option<String>,
    },
    ConnectionUp(String),
    ConnectionDown(String),
    ConnectionDelete(String),
    WifiRadio(bool),
    Airplane(bool),
    VpnImport(String),
    WireguardCreate(network::WireguardSpec),
    BluetoothQuery,
    BluetoothPower(bool),
    BluetoothScan(bool),
    BluetoothPair(String),
    BluetoothConnect(String),
    BluetoothDisconnect(String),
    BluetoothRemove(String),
    PowerQuery,
    SetPowerProfile(String),
    Logind(LogindAction),
    UsersQuery,
    SetRealName {
        user: String,
        name: String,
    },
    SetUserLocked {
        user: String,
        locked: bool,
    },
    ServicesQuery {
        user: bool,
    },
    UnitAction {
        user: bool,
        unit: String,
        action: UnitAction,
    },
    DisplayQuery {
        night_light: bool,
        night_temp: u32,
    },
    ApplyMonitor {
        name: String,
        mode: String,
        position: String,
        scale: f32,
        transform: u32,
        disabled: bool,
    },
    NightLight {
        on: bool,
        temp: u32,
    },
    InputLayouts,
    ApplyInput {
        kb_layout: String,
        kb_variant: String,
        kb_options: String,
        repeat_rate: u32,
        repeat_delay: u32,
        natural_scroll: bool,
        tap_to_click: bool,
        sensitivity: f32,
    },
    PrivacyQuery,
    TorMode(String),
    TorNewnym,
    TorBridges {
        kind: String,
        lines: String,
    },
    TailscaleUp,
    TailscaleDown,
    TailscaleLogin,
    TailscaleExitNode(Option<String>),
    TailscaleAllowLan(bool),
    TailscaleAdvertiseExit(bool),
    FprintQuery {
        user: String,
    },
    FprintEnroll {
        user: String,
        finger: String,
    },
    FprintDeleteAll {
        user: String,
    },
    AboutQuery {
        gpu: Option<String>,
    },
}

/// Results delivered to the sink.
#[derive(Clone, Debug)]
pub enum SysReply {
    Audio(audio::AudioState),
    Brightness(brightness::BrightnessState),
    Network(network::NetworkState),
    Bluetooth(bluetooth::BluetoothState),
    Power(power::PowerState),
    Users(Vec<users::UserInfo>),
    Services(services::ServicesState),
    Display(display::DisplayState),
    InputLayouts(Vec<input::LayoutInfo>),
    Privacy(privacy::PrivacyState),
    Fprint(fprint::FprintState),
    FprintProgress(String),
    About(about::AboutInfo),
    TailscaleLoginUrl(String),
    /// A mutating request finished; `refresh` names the query to re-run.
    Done {
        what: String,
        ok: bool,
        message: String,
    },
}

pub struct SystemBackend {
    tx: mpsc::Sender<SysRequest>,
}

impl SystemBackend {
    /// Spawn the worker. `sink` is invoked on the worker thread for every reply.
    pub fn spawn(
        runner: Arc<dyn CommandRunner>,
        sink: Arc<dyn Fn(SysReply) + Send + Sync>,
    ) -> Self {
        let (tx, rx) = mpsc::channel::<SysRequest>();
        thread::Builder::new()
            .name("edex-system".into())
            .spawn(move || {
                let comp = CompSocket::from_env();
                while let Ok(first) = rx.recv() {
                    let mut pending = vec![first];
                    pending.extend(rx.try_iter());
                    for req in schedule(pending) {
                        for r in handle(&*runner, comp.as_ref(), req, &sink) {
                            sink(r);
                        }
                    }
                }
            })
            .expect("spawn system worker");
        Self { tx }
    }

    pub fn send(&self, req: SysRequest) {
        let _ = self.tx.send(req);
    }
}

impl SysRequest {
    /// Read-only status refreshes, which panels send periodically.
    fn is_query(&self) -> bool {
        matches!(
            self,
            SysRequest::AudioQuery
                | SysRequest::BrightnessQuery
                | SysRequest::NetworkQuery { .. }
                | SysRequest::BluetoothQuery
                | SysRequest::PowerQuery
                | SysRequest::UsersQuery
                | SysRequest::ServicesQuery { .. }
                | SysRequest::DisplayQuery { .. }
                | SysRequest::PrivacyQuery
                | SysRequest::FprintQuery { .. }
                | SysRequest::AboutQuery { .. }
        )
    }
}

/// Order a batch of waiting requests: user actions first, in the order they were made, then each
/// kind of status query once (its latest copy). Periodic queries that arrive faster than they run
/// (slow machines, VMs) would otherwise pile up and delay actions behind them indefinitely.
fn schedule(pending: Vec<SysRequest>) -> Vec<SysRequest> {
    let (queries, mut out): (Vec<_>, Vec<_>) = pending.into_iter().partition(|r| r.is_query());
    let mut seen = Vec::new();
    let mut latest: Vec<SysRequest> = Vec::new();
    for q in queries.into_iter().rev() {
        let kind = std::mem::discriminant(&q);
        if !seen.contains(&kind) {
            seen.push(kind);
            latest.push(q);
        }
    }
    latest.reverse();
    out.extend(latest);
    out
}

fn done(what: &str, res: anyhow::Result<()>) -> SysReply {
    match res {
        Ok(()) => SysReply::Done {
            what: what.into(),
            ok: true,
            message: String::new(),
        },
        Err(e) => SysReply::Done {
            what: what.into(),
            ok: false,
            message: format!("{e:#}"),
        },
    }
}

fn handle(
    r: &dyn CommandRunner,
    comp: Option<&CompSocket>,
    req: SysRequest,
    sink: &Arc<dyn Fn(SysReply) + Send + Sync>,
) -> Vec<SysReply> {
    use SysRequest as Q;
    match req {
        Q::AudioQuery => vec![SysReply::Audio(audio::query(r))],
        Q::AudioSetVolume(v) => vec![
            done("audio", audio::set_volume(r, v)),
            SysReply::Audio(audio::query(r)),
        ],
        Q::AudioAdjustVolume(d) => vec![
            done("audio", audio::adjust_volume(r, d)),
            SysReply::Audio(audio::query(r)),
        ],
        Q::AudioToggleMute => vec![
            done("audio", audio::toggle_mute(r)),
            SysReply::Audio(audio::query(r)),
        ],
        Q::AudioToggleMicMute => vec![
            done("audio", audio::toggle_mic_mute(r)),
            SysReply::Audio(audio::query(r)),
        ],
        Q::AudioSetDefault(id) => vec![
            done("audio", audio::set_default(r, id)),
            SysReply::Audio(audio::query(r)),
        ],
        Q::BrightnessQuery => vec![SysReply::Brightness(brightness::query())],
        Q::BrightnessAdjust(d) => match brightness::adjust(d) {
            Ok(s) => vec![SysReply::Brightness(s)],
            Err(e) => vec![done("brightness", Err(e))],
        },
        Q::BrightnessSet(p) => match brightness::set(p) {
            Ok(s) => vec![SysReply::Brightness(s)],
            Err(e) => vec![done("brightness", Err(e))],
        },
        Q::NetworkQuery { rescan } => vec![SysReply::Network(network::query(rescan))],
        Q::WifiConnect { ssid, password } => vec![
            done("wifi", network::wifi_connect(&ssid, password.as_deref())),
            SysReply::Network(network::query(false)),
        ],
        Q::ConnectionUp(n) => vec![
            done("connection", network::connection_up(&n)),
            SysReply::Network(network::query(false)),
        ],
        Q::ConnectionDown(n) => vec![
            done("connection", network::connection_down(&n)),
            SysReply::Network(network::query(false)),
        ],
        Q::ConnectionDelete(n) => vec![
            done("connection", network::connection_delete(&n)),
            SysReply::Network(network::query(false)),
        ],
        Q::WifiRadio(on) => vec![
            done("wifi", network::wifi_radio(on)),
            SysReply::Network(network::query(false)),
        ],
        Q::Airplane(on) => vec![
            done("airplane", network::airplane(on)),
            SysReply::Network(network::query(false)),
        ],
        Q::WireguardCreate(spec) => vec![
            done("wireguard", network::wireguard_create(&spec).map(|_| ())),
            SysReply::Network(network::query(false)),
        ],
        Q::VpnImport(path) => vec![
            done("vpn-import", network::vpn_import(&path).map(|_| ())),
            SysReply::Network(network::query(false)),
        ],
        Q::BluetoothQuery => vec![SysReply::Bluetooth(bluetooth::query())],
        Q::BluetoothPower(on) => vec![
            done("bluetooth", bluetooth::power(on)),
            SysReply::Bluetooth(bluetooth::query()),
        ],
        Q::BluetoothScan(on) => vec![
            done("bluetooth", bluetooth::scan(on)),
            SysReply::Bluetooth(bluetooth::query()),
        ],
        Q::BluetoothPair(m) => vec![
            done("bluetooth", bluetooth::pair(&m)),
            SysReply::Bluetooth(bluetooth::query()),
        ],
        Q::BluetoothConnect(m) => vec![
            done("bluetooth", bluetooth::connect(&m)),
            SysReply::Bluetooth(bluetooth::query()),
        ],
        Q::BluetoothDisconnect(m) => vec![
            done("bluetooth", bluetooth::disconnect(&m)),
            SysReply::Bluetooth(bluetooth::query()),
        ],
        Q::BluetoothRemove(m) => vec![
            done("bluetooth", bluetooth::remove(&m)),
            SysReply::Bluetooth(bluetooth::query()),
        ],
        Q::PowerQuery => vec![SysReply::Power(power::query(r))],
        Q::SetPowerProfile(p) => vec![
            done("power-profile", power::set_profile(r, &p)),
            SysReply::Power(power::query(r)),
        ],
        Q::Logind(a) => vec![done("logind", power::logind(a))],
        Q::UsersQuery => vec![SysReply::Users(users::query(r))],
        Q::SetRealName { user, name } => vec![
            done("user", users::set_real_name(r, &user, &name)),
            SysReply::Users(users::query(r)),
        ],
        Q::SetUserLocked { user, locked } => vec![
            done("user", users::set_locked(r, &user, locked)),
            SysReply::Users(users::query(r)),
        ],
        Q::ServicesQuery { user } => vec![SysReply::Services(services::query(r, user))],
        Q::UnitAction { user, unit, action } => vec![
            done("unit", services::act(r, user, &unit, action)),
            SysReply::Services(services::query(r, user)),
        ],
        Q::DisplayQuery {
            night_light,
            night_temp,
        } => vec![SysReply::Display(display::query(
            comp,
            night_light,
            night_temp,
        ))],
        Q::ApplyMonitor {
            name,
            mode,
            position,
            scale,
            transform,
            disabled,
        } => {
            // The settings panel saved the rule in config.toml; edex-comp applies it.
            let _ = (name, mode, position, scale, transform, disabled);
            vec![done("monitor", display::reload(comp))]
        }
        Q::NightLight { on, temp } => {
            let _ = (on, temp);
            vec![done("night-light", display::reload(comp))]
        }
        Q::InputLayouts => vec![SysReply::InputLayouts(input::layouts(
            "/usr/share/X11/xkb/rules/evdev.xml",
        ))],
        Q::ApplyInput { .. } => {
            // Saved in config.toml by the settings panel; edex-comp applies it.
            vec![done("input", display::reload(comp))]
        }
        Q::PrivacyQuery => vec![SysReply::Privacy(privacy::query(r))],
        Q::TorMode(m) => vec![
            done("tor", privacy::tor_set_mode(r, &m)),
            SysReply::Privacy(privacy::query(r)),
        ],
        Q::TorNewnym => vec![done("tor", privacy::tor_newnym())],
        Q::TorBridges { kind, lines } => vec![
            done("tor-bridges", privacy::tor_bridges(r, &kind, &lines)),
            SysReply::Privacy(privacy::query(r)),
        ],
        Q::TailscaleUp => vec![
            done("tailscale", privacy::tailscale_up(r)),
            SysReply::Privacy(privacy::query(r)),
        ],
        Q::TailscaleDown => vec![
            done("tailscale", privacy::tailscale_down(r)),
            SysReply::Privacy(privacy::query(r)),
        ],
        Q::TailscaleLogin => match privacy::tailscale_login(r) {
            Ok(url) => vec![SysReply::TailscaleLoginUrl(url)],
            Err(e) => vec![done("tailscale-login", Err(e))],
        },
        Q::TailscaleExitNode(n) => vec![
            done("tailscale", privacy::tailscale_exit_node(r, n.as_deref())),
            SysReply::Privacy(privacy::query(r)),
        ],
        Q::TailscaleAllowLan(b) => vec![
            done("tailscale", privacy::tailscale_allow_lan(r, b)),
            SysReply::Privacy(privacy::query(r)),
        ],
        Q::TailscaleAdvertiseExit(b) => vec![
            done("tailscale", privacy::tailscale_advertise_exit(r, b)),
            SysReply::Privacy(privacy::query(r)),
        ],
        Q::FprintQuery { user } => vec![SysReply::Fprint(fprint::query(&user))],
        Q::FprintEnroll { user, finger } => {
            let s2 = sink.clone();
            let res = fprint::enroll(&user, &finger, &move |p| s2(SysReply::FprintProgress(p)));
            vec![done("fprint", res), SysReply::Fprint(fprint::query(&user))]
        }
        Q::FprintDeleteAll { user } => vec![
            done("fprint", fprint::delete_all(&user)),
            SysReply::Fprint(fprint::query(&user)),
        ],
        Q::AboutQuery { gpu } => {
            let hv = comp
                .and_then(|s| s.version().ok())
                .unwrap_or_else(|| "not connected".into());
            vec![SysReply::About(about::query(gpu, hv))]
        }
    }
}

#[cfg(test)]
mod schedule_tests {
    use super::*;

    #[test]
    fn actions_first_and_queries_coalesced() {
        let batch = vec![
            SysRequest::PrivacyQuery,
            SysRequest::NetworkQuery { rescan: false },
            SysRequest::PrivacyQuery,
            SysRequest::VpnImport("a.conf".into()),
            SysRequest::NetworkQuery { rescan: true },
            SysRequest::PrivacyQuery,
            SysRequest::TorNewnym,
        ];
        let out = schedule(batch);
        let names: Vec<String> = out.iter().map(|r| format!("{r:?}")).collect();
        assert_eq!(
            names,
            [
                "VpnImport(\"a.conf\")",
                "TorNewnym",
                "NetworkQuery { rescan: true }",
                "PrivacyQuery",
            ]
        );
    }
}
