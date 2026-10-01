//! The privacy overlay: Tor, Tailscale, VPN, DNS/firewall and device indicators.

use platform::Platform;
use system::SysRequest;
use ui::form::Form;

use super::{build::*, Change};
use crate::{app::App, events::AppEvent};

pub const TAB_TOR: usize = 0;
pub const TAB_TAILSCALE: usize = 1;
pub const TAB_VPN: usize = 2;
pub const TAB_DNS: usize = 3;
pub const TAB_DEVICES: usize = 4;
const TABS: [&str; 5] = ["Tor", "Tailscale", "VPN", "DNS & Firewall", "Devices"];

mod id {
    pub const TOR_STATUS: u32 = 1;
    pub const TOR_MODE: u32 = 2;
    pub const TOR_BOOT: u32 = 3;
    pub const TOR_CIRCUIT: u32 = 4;
    pub const TOR_NEWNYM: u32 = 5;
    pub const TOR_BRIDGES: u32 = 6;
    pub const TOR_OBFS4: u32 = 7;
    pub const TOR_APPLY_BRIDGES: u32 = 8;
    pub const TOR_ON_LOGIN: u32 = 9;
    pub const TOR_CONFIRM: u32 = 10;
    pub const TS_STATE: u32 = 101;
    pub const TS_LOGIN: u32 = 102;
    pub const TS_UP: u32 = 103;
    pub const TS_DOWN: u32 = 104;
    pub const TS_EXIT: u32 = 105;
    pub const TS_LAN: u32 = 106;
    pub const TS_ADVERTISE: u32 = 107;
    pub const TS_PEERS: u32 = 108;
    pub const VPN_LIST: u32 = 201;
    pub const VPN_PATH: u32 = 202;
    pub const VPN_IMPORT: u32 = 203;
    pub const VPN_DELETE: u32 = 204;
    pub const WG_LIST: u32 = 210;
    pub const WG_NAME: u32 = 211;
    pub const WG_ADDRESS: u32 = 212;
    pub const WG_DNS: u32 = 213;
    pub const WG_PEER_KEY: u32 = 214;
    pub const WG_ENDPOINT: u32 = 215;
    pub const WG_ALLOWED: u32 = 216;
    pub const WG_KEEPALIVE: u32 = 217;
    pub const WG_CREATE: u32 = 218;
    pub const WG_DELETE: u32 = 219;
    pub const DNS_TEST: u32 = 301;
    pub const FP_ENROLLED: u32 = 401;
}

#[derive(Default)]
pub struct PrivacyScratch {
    pub obfs4: String,
    pub bridges: usize,
    pub vpn_path: String,
    pub pending_transparent: bool,
    /// New WireGuard tunnel being filled in.
    pub wg: system::network::WireguardSpec,
}

const MODES: [&str; 3] = ["off", "socks5", "transparent"];
const BRIDGES: [&str; 3] = ["none", "snowflake", "obfs4"];

pub fn open(app: &mut App) {
    let f = &mut app.state.privacy;
    f.title = "PRIVACY".into();
    f.tabs = TABS.iter().map(|s| s.to_string()).collect();
    f.status = None;
    app.pscratch.bridges = BRIDGES
        .iter()
        .position(|b| *b == app.sys.privacy.tor.bridges)
        .unwrap_or(0);
    app.system.send(SysRequest::PrivacyQuery);
    app.system.send(SysRequest::NetworkQuery { rescan: false });
    app.system.send(SysRequest::FprintQuery {
        user: app.state.username.clone(),
    });
    rebuild(app);
}

pub fn select_tab(app: &mut App, tab: usize) {
    let f = &mut app.state.privacy;
    f.active = tab.min(TABS.len() - 1);
    f.form_state = Default::default();
    rebuild(app);
}

pub fn rebuild(app: &mut App) {
    let form = match app.state.privacy.active {
        TAB_TOR => tor(app),
        TAB_TAILSCALE => tailscale(app),
        TAB_VPN => vpn(app),
        TAB_DNS => dns(app),
        _ => devices(app),
    };
    let f = &mut app.state.privacy;
    if let Some(editing) = f.form_state.editing {
        if let Some(old) = f.form.control(editing).cloned() {
            let mut form = form;
            if let Some(c) = form.control_mut(editing) {
                c.kind = old.kind;
            }
            f.form = form;
            app.mark_overlay_dirty();
            return;
        }
    }
    f.form = form;
    app.mark_overlay_dirty();
}

fn tor(app: &App) -> Form {
    let t = &app.sys.privacy.tor;
    let s = &app.pscratch;
    let status = if !t.installed {
        "tor is not installed".to_string()
    } else if t.running {
        format!(
            "running{}",
            if t.version.is_empty() {
                String::new()
            } else {
                format!(" (v{})", t.version)
            }
        )
    } else {
        "not running".into()
    };
    let mut mode = vec![
        info(id::TOR_STATUS, "Daemon", status),
        choice(
            id::TOR_MODE,
            "Mode",
            &MODES,
            MODES.iter().position(|m| *m == t.mode).unwrap_or(0),
        ),
    ];
    if s.pending_transparent {
        mode.push(button(id::TOR_CONFIRM, "Transparent mode routes ALL traffic through Tor and blocks anything that cannot be. Confirm?", "ENABLE TRANSPARENT"));
    }
    mode.extend([
        progress(
            id::TOR_BOOT,
            "Bootstrap",
            t.bootstrap_pct as f32 / 100.0,
            &format!("{}%", t.bootstrap_pct),
        ),
        info(
            id::TOR_CIRCUIT,
            "Circuit",
            if t.circuit_established {
                "established"
            } else {
                "not established"
            },
        ),
        button(
            id::TOR_NEWNYM,
            "Request a new identity (NEWNYM)",
            "NEW IDENTITY",
        ),
        toggle(
            id::TOR_ON_LOGIN,
            "Restore Tor mode at login",
            app.config.privacy.tor_mode_on_login,
        ),
    ]);
    Form::default()
        .section("Tor", mode)
        .section("Bridges", vec![
            choice(id::TOR_BRIDGES, "Pluggable transport", &BRIDGES, s.bridges),
            text(id::TOR_OBFS4, "obfs4 bridge lines (separate with ;)", &s.obfs4, "obfs4 1.2.3.4:443 FINGERPRINT cert=… iat-mode=0"),
            button(id::TOR_APPLY_BRIDGES, "Write bridges and reload tor", "APPLY"),
            note(0, "socks5: apps configured for 127.0.0.1:9050 use Tor.\ntransparent: everything except Tailscale and the LAN is redirected through Tor; the firewall fails closed."),
        ])
}

fn tailscale(app: &App) -> Form {
    let t = &app.sys.privacy.tailscale;
    let peers = t
        .peers
        .iter()
        .map(|p| {
            item(
                p.name.clone(),
                format!(
                    "{}  {}{}",
                    p.ip,
                    p.os,
                    if p.exit_node_option {
                        "  exit-node"
                    } else {
                        ""
                    }
                ),
                p.exit_node_active,
                Some(if p.online { "online" } else { "offline" }),
            )
        })
        .collect();
    let mut exits = vec!["none".to_string()];
    exits.extend(
        t.peers
            .iter()
            .filter(|p| p.exit_node_option)
            .map(|p| p.name.clone()),
    );
    let esel = t
        .exit_node
        .as_ref()
        .and_then(|e| exits.iter().position(|x| x == e))
        .unwrap_or(0);
    let state = if !t.installed {
        "tailscale is not installed".to_string()
    } else {
        format!(
            "{}{}",
            t.backend_state,
            if t.self_ip.is_empty() {
                String::new()
            } else {
                format!("  {} ({})", t.self_name, t.self_ip)
            }
        )
    };
    Form::default()
        .section(
            "Tailscale",
            vec![
                info(id::TS_STATE, "State", state),
                info(
                    0,
                    "Tailnet",
                    if t.tailnet.is_empty() {
                        "-".into()
                    } else {
                        t.tailnet.clone()
                    },
                ),
                button(id::TS_LOGIN, "Log in (opens the browser)", "LOGIN"),
                button(id::TS_UP, "Connect", "UP"),
                button(id::TS_DOWN, "Disconnect", "DOWN"),
            ],
        )
        .section(
            "Exit node",
            vec![
                choice_owned(id::TS_EXIT, "Use exit node", exits, esel),
                toggle(
                    id::TS_LAN,
                    "Allow LAN access while using an exit node",
                    t.allow_lan,
                ),
                toggle(
                    id::TS_ADVERTISE,
                    "Advertise this machine as an exit node",
                    t.advertise_exit,
                ),
            ],
        )
        .section(
            "Peers",
            vec![list(id::TS_PEERS, "Devices", peers, None, "no peers")],
        )
}

fn wireguard_tunnels(app: &App) -> Vec<&system::network::Connection> {
    app.sys
        .network
        .vpn_connections
        .iter()
        .filter(|c| c.kind == "wireguard")
        .collect()
}

fn other_vpns(app: &App) -> Vec<&system::network::Connection> {
    app.sys
        .network
        .vpn_connections
        .iter()
        .filter(|c| c.kind != "wireguard")
        .collect()
}

fn vpn(app: &App) -> Form {
    let conn_item = |c: &&system::network::Connection| {
        item(
            c.name.clone(),
            c.kind.clone(),
            c.active,
            if c.active { Some("connected") } else { None },
        )
    };
    let tunnels = wireguard_tunnels(app);
    let wg_items = tunnels.iter().map(conn_item).collect();
    let selected = tunnels.get(app.state.privacy.form_state.list_cursor(id::WG_LIST));
    let public_key = selected
        .map(|c| {
            if c.wg_public_key.is_empty() {
                "(not readable)".to_string()
            } else {
                c.wg_public_key.clone()
            }
        })
        .unwrap_or_else(|| "-".into());
    let w = &app.pscratch.wg;
    let other_items = other_vpns(app).iter().map(conn_item).collect();
    Form::default()
        .section(
            "WireGuard",
            vec![
                list(
                    id::WG_LIST,
                    "Tunnels",
                    wg_items,
                    Some("TOGGLE"),
                    "no tunnels yet: create one below or import a .conf",
                ),
                info(
                    0,
                    "Public key of the selected tunnel (add it on the server)",
                    public_key,
                ),
                button(id::WG_DELETE, "Selected tunnel", "DELETE"),
            ],
        )
        .section(
            "New WireGuard tunnel",
            vec![
                text(id::WG_NAME, "Name (interface)", &w.name, "wg-home"),
                text(id::WG_ADDRESS, "Address", &w.address, "10.8.0.2/32"),
                text(id::WG_DNS, "DNS (optional)", &w.dns, "10.8.0.1"),
                text(
                    id::WG_PEER_KEY,
                    "Server public key",
                    &w.peer_public_key,
                    "44-character base64 key",
                ),
                text(
                    id::WG_ENDPOINT,
                    "Server endpoint",
                    &w.endpoint,
                    "vpn.example.com:51820",
                ),
                text(
                    id::WG_ALLOWED,
                    "Allowed IPs",
                    &w.allowed_ips,
                    "0.0.0.0/0, ::/0 (all traffic)",
                ),
                text(
                    id::WG_KEEPALIVE,
                    "Keepalive seconds (optional)",
                    &if w.keepalive > 0 {
                        w.keepalive.to_string()
                    } else {
                        String::new()
                    },
                    "25",
                ),
                button(id::WG_CREATE, "Generate keys and create", "CREATE"),
            ],
        )
        .section(
            "Import and other VPNs (NetworkManager)",
            vec![
                list(
                    id::VPN_LIST,
                    "OpenVPN and other connections",
                    other_items,
                    Some("TOGGLE"),
                    "no other VPN connections",
                ),
                text(
                    id::VPN_PATH,
                    "Import file",
                    &app.pscratch.vpn_path,
                    "/path/to/config.ovpn or wg0.conf",
                ),
                button(id::VPN_IMPORT, "Import OpenVPN / WireGuard file", "IMPORT"),
                button(id::VPN_DELETE, "Selected connection", "DELETE"),
            ],
        )
}

fn dns(app: &App) -> Form {
    let d = &app.sys.privacy.dns;
    Form::default()
        .section("DNS", vec![
            info(0, "Resolver", d.resolver.clone()),
            info(0, "dnscrypt-proxy", if d.dnscrypt_active { "active (encrypted DNS)" } else { "not active" }),
            info(0, "Test query", d.test_result.clone()),
            button(id::DNS_TEST, "Resolve check.torproject.org", "TEST"),
        ])
        .section("Firewall", vec![
            info(0, "nftables", if app.sys.privacy.firewall_active { "active" } else { "inactive" }),
            note(0, "Inbound connections are dropped except on the Tailscale interface. Tor transparent mode adds a fail-closed output policy."),
        ])
}

fn devices(app: &App) -> Form {
    let s = &app.state.status;
    let f = &app.sys.fprint;
    let enrolled = f
        .enrolled
        .iter()
        .map(|e| item(e.clone(), "", false, None))
        .collect();
    Form::default()
        .section(
            "Live indicators",
            vec![
                info(
                    0,
                    "Microphone",
                    if s.mic_active { "IN USE" } else { "idle" },
                ),
                info(0, "Camera", if s.camera_active { "IN USE" } else { "idle" }),
                info(
                    0,
                    "Fingerprint daemon",
                    if s.fprintd_active { "active" } else { "idle" },
                ),
                info(
                    0,
                    "Tor",
                    format!(
                        "{}{}",
                        s.tor_mode,
                        if s.tor_active {
                            " (daemon running)"
                        } else {
                            ""
                        }
                    ),
                ),
                info(
                    0,
                    "Tailscale",
                    if s.tailscale_active {
                        "connected"
                    } else {
                        "disconnected"
                    },
                ),
                info(0, "VPN", if s.vpn_active { "active" } else { "inactive" }),
            ],
        )
        .section(
            "Fingerprint",
            vec![
                info(
                    0,
                    "Reader",
                    if f.available {
                        f.device.clone()
                    } else {
                        "none".into()
                    },
                ),
                list(
                    id::FP_ENROLLED,
                    "Enrolled fingers",
                    enrolled,
                    None,
                    "none — enrol under Settings → Security",
                ),
            ],
        )
}

pub fn on_change(app: &mut App, platform: &mut Platform<AppEvent>, id: u32, ch: Change) {
    match id {
        id::TOR_MODE => {
            if let Change::Choice(i) = ch {
                let mode = MODES[i.min(2)];
                if mode == "transparent" {
                    app.pscratch.pending_transparent = true;
                } else {
                    app.pscratch.pending_transparent = false;
                    app.state.privacy.status = Some(format!("switching Tor to {mode}…"));
                    app.system.send(SysRequest::TorMode(mode.into()));
                }
            }
        }
        id::TOR_CONFIRM => {
            app.pscratch.pending_transparent = false;
            app.state.privacy.status =
                Some("enabling transparent mode (waits for bootstrap)…".into());
            app.system.send(SysRequest::TorMode("transparent".into()));
        }
        id::TOR_NEWNYM => app.system.send(SysRequest::TorNewnym),
        id::TOR_ON_LOGIN => {
            if let Change::Toggle(v) = ch {
                app.config.privacy.tor_mode_on_login = v;
                app.commit_config(platform, false);
            }
        }
        id::TOR_BRIDGES => {
            if let Change::Choice(i) = ch {
                app.pscratch.bridges = i;
            }
        }
        id::TOR_OBFS4 => {
            if let Change::Text(t) = ch {
                app.pscratch.obfs4 = t;
            }
        }
        id::TOR_APPLY_BRIDGES => {
            let kind = match app.pscratch.bridges {
                1 => "snowflake",
                2 => "obfs4",
                _ => "clear",
            };
            let lines = app
                .pscratch
                .obfs4
                .split(';')
                .map(|l| l.trim())
                .filter(|l| !l.is_empty())
                .collect::<Vec<_>>()
                .join("\n");
            app.system.send(SysRequest::TorBridges {
                kind: kind.into(),
                lines,
            });
        }
        id::TS_LOGIN => app.system.send(SysRequest::TailscaleLogin),
        id::TS_UP => app.system.send(SysRequest::TailscaleUp),
        id::TS_DOWN => app.system.send(SysRequest::TailscaleDown),
        id::TS_EXIT => {
            if let Change::Choice(i) = ch {
                let t = &app.sys.privacy.tailscale;
                let exits: Vec<String> = t
                    .peers
                    .iter()
                    .filter(|p| p.exit_node_option)
                    .map(|p| p.name.clone())
                    .collect();
                let node = if i == 0 {
                    None
                } else {
                    exits.get(i - 1).cloned()
                };
                app.config.privacy.tailscale_exit_node = node.clone().unwrap_or_default();
                app.commit_config(platform, false);
                app.system.send(SysRequest::TailscaleExitNode(node));
            }
        }
        id::TS_LAN => {
            if let Change::Toggle(v) = ch {
                app.system.send(SysRequest::TailscaleAllowLan(v));
            }
        }
        id::TS_ADVERTISE => {
            if let Change::Toggle(v) = ch {
                app.system.send(SysRequest::TailscaleAdvertiseExit(v));
            }
        }
        id::VPN_LIST | id::WG_LIST => {
            if let Change::ListAction(i) = ch {
                let list = if id == id::WG_LIST {
                    wireguard_tunnels(app)
                } else {
                    other_vpns(app)
                };
                if let Some(c) = list.get(i) {
                    let req = if c.active {
                        SysRequest::ConnectionDown(c.name.clone())
                    } else {
                        SysRequest::ConnectionUp(c.name.clone())
                    };
                    app.system.send(req);
                }
            }
        }
        id::VPN_PATH => {
            if let Change::Text(t) = ch {
                app.pscratch.vpn_path = t;
            }
        }
        id::VPN_IMPORT => {
            let p = app.pscratch.vpn_path.trim().to_string();
            if !p.is_empty() {
                app.system.send(SysRequest::VpnImport(p));
                app.pscratch.vpn_path.clear();
            }
        }
        id::VPN_DELETE | id::WG_DELETE => {
            let (list, list_id) = if id == id::WG_DELETE {
                (wireguard_tunnels(app), id::WG_LIST)
            } else {
                (other_vpns(app), id::VPN_LIST)
            };
            let cur = app.state.privacy.form_state.list_cursor(list_id);
            if let Some(name) = list.get(cur).map(|c| c.name.clone()) {
                app.system.send(SysRequest::ConnectionDelete(name));
            }
        }
        id::WG_NAME
        | id::WG_ADDRESS
        | id::WG_DNS
        | id::WG_PEER_KEY
        | id::WG_ENDPOINT
        | id::WG_ALLOWED
        | id::WG_KEEPALIVE => {
            if let Change::Text(t) = ch {
                let w = &mut app.pscratch.wg;
                match id {
                    id::WG_NAME => w.name = t,
                    id::WG_ADDRESS => w.address = t,
                    id::WG_DNS => w.dns = t,
                    id::WG_PEER_KEY => w.peer_public_key = t,
                    id::WG_ENDPOINT => w.endpoint = t,
                    id::WG_ALLOWED => w.allowed_ips = t,
                    _ => w.keepalive = t.trim().parse().unwrap_or(0),
                }
            }
        }
        id::WG_CREATE => match app.pscratch.wg.validate() {
            Ok(()) => {
                app.system
                    .send(SysRequest::WireguardCreate(app.pscratch.wg.clone()));
                app.pscratch.wg = Default::default();
            }
            Err(e) => app.state.privacy.status = Some(format!("WireGuard: {e}")),
        },
        id::DNS_TEST => app.system.send(SysRequest::PrivacyQuery),
        _ => {}
    }
}
