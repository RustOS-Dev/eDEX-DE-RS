//! The 14-category settings overlay: builds forms from config + backend state and applies edits.

use platform::Platform;
use system::{SysRequest, UnitAction};
use ui::{form::Form, state::TabbedForms};

use super::{build::*, Change};
use crate::{app::App, events::AppEvent};

pub const TAB_APPEARANCE: usize = 0;
pub const TAB_DISPLAY: usize = 1;
pub const TAB_INPUT: usize = 2;
pub const TAB_AUDIO: usize = 3;
pub const TAB_NETWORK: usize = 4;
pub const TAB_BLUETOOTH: usize = 5;
pub const TAB_POWER: usize = 6;
pub const TAB_SECURITY: usize = 7;
pub const TAB_USERS: usize = 8;
pub const TAB_NOTIFICATIONS: usize = 9;
pub const TAB_SERVICES: usize = 10;
pub const TAB_WM: usize = 11;
pub const TAB_TERMINAL: usize = 12;
pub const TAB_ABOUT: usize = 13;

const TABS: [&str; 14] = [
    "Appearance",
    "Display",
    "Input",
    "Audio",
    "Network",
    "Bluetooth",
    "Power",
    "Security",
    "Users",
    "Notifications",
    "Services",
    "Window Manager",
    "Terminal",
    "About",
];

// Control ids: tab * 100 + n.
mod id {
    pub const THEME: u32 = 1;
    pub const FONT: u32 = 2;
    pub const FONT_SIZE: u32 = 3;
    pub const GLOW: u32 = 4;
    pub const SCANLINES: u32 = 5;
    pub const ANIMATIONS: u32 = 6;
    pub const KEYBOARD: u32 = 7;
    pub const BOOT: u32 = 8;
    pub const THEME_APPS: u32 = 1301;
    pub const SIDE_PANELS: u32 = 9;
    pub const FS_SPLIT: u32 = 10;
    pub const SYS_SPLIT: u32 = 11;
    pub const MONITORS: u32 = 101;
    pub const MON_MODE: u32 = 102;
    pub const MON_POS: u32 = 103;
    pub const MON_SCALE: u32 = 104;
    pub const MON_TRANSFORM: u32 = 105;
    pub const MON_DISABLED: u32 = 106;
    pub const MON_APPLY: u32 = 107;
    pub const NIGHT: u32 = 108;
    pub const NIGHT_TEMP: u32 = 109;
    pub const KB_LAYOUT: u32 = 201;
    pub const KB_VARIANT: u32 = 202;
    pub const KB_OPTIONS: u32 = 203;
    pub const REPEAT_RATE: u32 = 204;
    pub const REPEAT_DELAY: u32 = 205;
    pub const NATURAL: u32 = 206;
    pub const TAP: u32 = 207;
    pub const SENS: u32 = 208;
    pub const VOLUME: u32 = 301;
    pub const MUTE: u32 = 302;
    pub const MIC_MUTE: u32 = 303;
    pub const SINKS: u32 = 304;
    pub const SOURCES: u32 = 305;
    pub const MIXER: u32 = 306;
    pub const WIFI: u32 = 401;
    pub const AIRPLANE: u32 = 402;
    pub const RESCAN: u32 = 403;
    pub const NETWORKS: u32 = 404;
    pub const PASSWORD: u32 = 405;
    pub const CONNECTIONS: u32 = 406;
    pub const CONN_DELETE: u32 = 407;
    pub const VPN_PATH: u32 = 408;
    pub const VPN_IMPORT: u32 = 409;
    pub const BT_POWER: u32 = 501;
    pub const BT_SCAN: u32 = 502;
    pub const BT_DEVICES: u32 = 503;
    pub const BT_PAIR: u32 = 504;
    pub const BT_REMOVE: u32 = 505;
    pub const BRIGHTNESS: u32 = 601;
    pub const PROFILE: u32 = 602;
    pub const DIM: u32 = 603;
    pub const LOCK: u32 = 604;
    pub const DPMS: u32 = 605;
    pub const SUSPEND: u32 = 606;
    pub const LID: u32 = 607;
    pub const LOCK_SLEEP: u32 = 608;
    pub const BATTERY: u32 = 609;
    pub const FP_DEVICE: u32 = 701;
    pub const FP_ENROLLED: u32 = 702;
    pub const FP_FINGER: u32 = 703;
    pub const FP_ENROLL: u32 = 704;
    pub const FP_DELETE: u32 = 705;
    pub const FP_PROGRESS: u32 = 706;
    pub const FIREWALL: u32 = 707;
    pub const KEYRING: u32 = 708;
    pub const LOCK_NOW: u32 = 709;
    pub const FP_LOGIN: u32 = 710;
    pub const USERS: u32 = 801;
    pub const REAL_NAME: u32 = 802;
    pub const SET_NAME: u32 = 803;
    pub const PASSWD: u32 = 804;
    pub const LOCK_USER: u32 = 805;
    pub const DND: u32 = 901;
    pub const TIMEOUT: u32 = 902;
    pub const POSITION: u32 = 903;
    pub const MAX_VISIBLE: u32 = 904;
    pub const MUTED: u32 = 905;
    pub const TEST: u32 = 906;
    pub const CLEAR: u32 = 907;
    pub const SCOPE: u32 = 1001;
    pub const FILTER: u32 = 1002;
    pub const UNITS: u32 = 1003;
    pub const ENABLE: u32 = 1004;
    pub const DISABLE: u32 = 1005;
    pub const RESTART: u32 = 1006;
    pub const GAPS_IN: u32 = 1101;
    pub const GAPS_OUT: u32 = 1102;
    pub const BORDER: u32 = 1103;
    pub const LAYOUT: u32 = 1104;
    pub const WORKSPACES: u32 = 1105;
    pub const WM_ANIM: u32 = 1106;
    pub const BLUR: u32 = 1107;
    pub const ROUNDING: u32 = 1108;
    pub const RELOAD: u32 = 1109;
    pub const BINDS: u32 = 1110;
    pub const SHELL: u32 = 1201;
    pub const SCROLLBACK: u32 = 1202;
    pub const TERM_FONT: u32 = 1203;
    pub const CURSOR: u32 = 1204;
    pub const BLINK: u32 = 1205;
    pub const BELL: u32 = 1206;
    pub const OSC52: u32 = 1207;
}

/// Per-overlay scratch values that are not part of the config.
#[derive(Default)]
pub struct SettingsScratch {
    pub wifi_password: String,
    pub vpn_path: String,
    pub real_name: String,
    pub service_filter: String,
    pub services_user: bool,
    pub finger: usize,
    pub monitor: usize,
    pub mon_mode: usize,
    pub mon_pos: String,
    pub mon_scale: f32,
    pub mon_transform: usize,
    pub mon_disabled: bool,
}

pub fn open(app: &mut App) {
    let forms = &mut app.state.settings;
    forms.title = "SETTINGS".into();
    forms.tabs = TABS.iter().map(|s| s.to_string()).collect();
    forms.status = None;
    request_for_tab(app, app.state.settings.active);
    rebuild(app);
}

pub fn select_tab(app: &mut App, tab: usize) {
    let forms = &mut app.state.settings;
    forms.active = tab.min(TABS.len() - 1);
    forms.form_state = Default::default();
    request_for_tab(app, tab);
    rebuild(app);
}

fn request_for_tab(app: &mut App, tab: usize) {
    let reqs: Vec<SysRequest> = match tab {
        TAB_DISPLAY => vec![SysRequest::DisplayQuery {
            night_light: app.config.display.night_light,
            night_temp: app.config.display.night_temp,
        }],
        TAB_INPUT => vec![SysRequest::InputLayouts],
        TAB_AUDIO => vec![SysRequest::AudioQuery],
        TAB_NETWORK => vec![SysRequest::NetworkQuery { rescan: false }],
        TAB_BLUETOOTH => vec![SysRequest::BluetoothQuery],
        TAB_POWER => vec![SysRequest::PowerQuery, SysRequest::BrightnessQuery],
        TAB_SECURITY => vec![
            SysRequest::FprintQuery {
                user: app.state.username.clone(),
            },
            SysRequest::PrivacyQuery,
        ],
        TAB_USERS => vec![SysRequest::UsersQuery],
        TAB_SERVICES => vec![SysRequest::ServicesQuery {
            user: app.scratch.services_user,
        }],
        TAB_ABOUT => vec![SysRequest::AboutQuery {
            gpu: app.gpu.adapter_info(),
        }],
        _ => vec![],
    };
    for r in reqs {
        app.system.send(r);
    }
}

pub fn rebuild(app: &mut App) {
    let tab = app.state.settings.active;
    let form = match tab {
        TAB_APPEARANCE => appearance(app),
        TAB_DISPLAY => display(app),
        TAB_INPUT => input(app),
        TAB_AUDIO => audio(app),
        TAB_NETWORK => network(app),
        TAB_BLUETOOTH => bluetooth(app),
        TAB_POWER => power(app),
        TAB_SECURITY => security(app),
        TAB_USERS => users(app),
        TAB_NOTIFICATIONS => notifications(app),
        TAB_SERVICES => services(app),
        TAB_WM => wm(app),
        TAB_TERMINAL => terminal(app),
        _ => about(app),
    };
    let forms: &mut TabbedForms = &mut app.state.settings;
    // Preserve in-progress text edits.
    if let Some(editing) = forms.form_state.editing {
        if let Some(old) = forms.form.control(editing).cloned() {
            let mut form = form;
            if let Some(c) = form.control_mut(editing) {
                c.kind = old.kind;
            }
            forms.form = form;
            app.mark_overlay_dirty();
            return;
        }
    }
    forms.form = form;
    app.mark_overlay_dirty();
}

fn index_of(options: &[&str], v: &str) -> usize {
    options.iter().position(|o| *o == v).unwrap_or(0)
}

// ─── Builders ───────────────────────────────────────────────────────────────

fn appearance(app: &App) -> Form {
    let c = &app.config;
    let themes: Vec<String> = app.themes.keys().cloned().collect();
    let sel = themes
        .iter()
        .position(|t| *t == c.appearance.theme)
        .unwrap_or(0);
    Form::default()
        .section(
            "Theme",
            vec![
                choice_owned(id::THEME, "Theme", themes, sel),
                text(
                    id::FONT,
                    "Font family",
                    &c.appearance.font,
                    "JetBrainsMono Nerd Font",
                ),
                slider(
                    id::FONT_SIZE,
                    "UI font size",
                    c.appearance.font_size,
                    8.0,
                    32.0,
                    1.0,
                    " px",
                ),
                slider(
                    id::GLOW,
                    "Border glow",
                    c.appearance.border_glow * 100.0,
                    0.0,
                    100.0,
                    10.0,
                    "%",
                ),
                toggle(id::SCANLINES, "Scanlines", c.appearance.scanlines),
                toggle(id::ANIMATIONS, "Animations", c.appearance.animations),
                toggle(id::BOOT, "Boot animation", c.appearance.boot_animation),
                toggle(
                    id::THEME_APPS,
                    "Apply theme to Qt and GTK apps",
                    c.appearance.theme_apps,
                ),
            ],
        )
        .section(
            "Layout",
            vec![
                toggle(
                    id::KEYBOARD,
                    "On-screen keyboard (touchscreens)",
                    c.appearance.keyboard_visible,
                ),
                toggle(
                    id::SIDE_PANELS,
                    "Keep side panels visible next to apps",
                    c.layout.reserve_side_panels,
                ),
                slider(
                    id::FS_SPLIT,
                    "File panel width",
                    c.layout.fs_split * 100.0,
                    8.0,
                    50.0,
                    1.0,
                    "%",
                ),
                slider(
                    id::SYS_SPLIT,
                    "System panel start",
                    c.layout.sysinfo_split * 100.0,
                    50.0,
                    95.0,
                    1.0,
                    "%",
                ),
            ],
        )
}

const TRANSFORMS: [&str; 8] = [
    "normal",
    "90°",
    "180°",
    "270°",
    "flipped",
    "flipped 90°",
    "flipped 180°",
    "flipped 270°",
];

fn display(app: &App) -> Form {
    let c = &app.config;
    let s = &app.scratch;
    let mons = &app.sys.display.monitors;
    let items = mons
        .iter()
        .enumerate()
        .map(|(i, m)| {
            item(
                m.name.clone(),
                format!(
                    "{}x{}@{:.0} at {},{} scale {:.2}{}",
                    m.width,
                    m.height,
                    m.refresh_rate,
                    m.x,
                    m.y,
                    m.scale,
                    if m.disabled { " (off)" } else { "" }
                ),
                i == s.monitor,
                if m.focused { Some("focused") } else { None },
            )
        })
        .collect();
    let mut form = Form::default().section(
        "Monitors",
        vec![list(
            id::MONITORS,
            "Outputs",
            items,
            None,
            if app.hypr.is_some() {
                "no monitors reported"
            } else {
                "Hyprland not connected"
            },
        )],
    );
    if let Some(m) = mons.get(s.monitor) {
        let mut modes = vec!["preferred".to_string(), "highrr".into(), "highres".into()];
        modes.extend(m.available_modes.iter().cloned());
        form = form.section(
            format!("Configure {}", m.name),
            vec![
                choice_owned(id::MON_MODE, "Mode", modes, s.mon_mode),
                text(id::MON_POS, "Position", &s.mon_pos, "auto or x,y"),
                slider(id::MON_SCALE, "Scale", s.mon_scale, 1.0, 3.0, 0.25, "x"),
                choice(id::MON_TRANSFORM, "Rotation", &TRANSFORMS, s.mon_transform),
                toggle(id::MON_DISABLED, "Disabled", s.mon_disabled),
                button(id::MON_APPLY, "Apply and save", "APPLY"),
            ],
        );
    }
    form.section(
        "Night light",
        vec![
            toggle(id::NIGHT, "Night light (hyprsunset)", c.display.night_light),
            slider(
                id::NIGHT_TEMP,
                "Colour temperature",
                c.display.night_temp as f32,
                1000.0,
                6500.0,
                100.0,
                " K",
            ),
        ],
    )
}

fn input(app: &App) -> Form {
    let c = &app.config;
    let layouts: Vec<String> = app
        .sys
        .layouts
        .iter()
        .map(|l| format!("{} — {}", l.code, l.description))
        .collect();
    let sel = app
        .sys
        .layouts
        .iter()
        .position(|l| l.code == c.input.kb_layout)
        .unwrap_or(0);
    Form::default()
        .section(
            "Keyboard",
            vec![
                if layouts.is_empty() {
                    text(id::KB_LAYOUT, "Layout", &c.input.kb_layout, "us")
                } else {
                    choice_owned(id::KB_LAYOUT, "Layout", layouts, sel)
                },
                text(
                    id::KB_VARIANT,
                    "Variant",
                    &c.input.kb_variant,
                    "e.g. dvorak",
                ),
                text(
                    id::KB_OPTIONS,
                    "Options",
                    &c.input.kb_options,
                    "e.g. caps:escape",
                ),
                slider(
                    id::REPEAT_RATE,
                    "Repeat rate",
                    c.input.repeat_rate as f32,
                    10.0,
                    100.0,
                    5.0,
                    "/s",
                ),
                slider(
                    id::REPEAT_DELAY,
                    "Repeat delay",
                    c.input.repeat_delay as f32,
                    100.0,
                    1000.0,
                    50.0,
                    " ms",
                ),
            ],
        )
        .section(
            "Pointer",
            vec![
                toggle(id::NATURAL, "Natural scrolling", c.input.natural_scroll),
                toggle(id::TAP, "Tap to click", c.input.tap_to_click),
                slider(
                    id::SENS,
                    "Sensitivity",
                    c.input.sensitivity,
                    -1.0,
                    1.0,
                    0.1,
                    "",
                ),
            ],
        )
}

fn audio(app: &App) -> Form {
    let a = &app.sys.audio;
    let sinks = a
        .sinks
        .iter()
        .map(|d| {
            item(
                d.name.clone(),
                format!("id {}", d.id),
                d.default,
                if d.default { Some("default") } else { None },
            )
        })
        .collect();
    let sources = a
        .sources
        .iter()
        .map(|d| {
            item(
                d.name.clone(),
                format!("id {}", d.id),
                d.default,
                if d.default { Some("default") } else { None },
            )
        })
        .collect();
    Form::default()
        .section(
            "Output",
            vec![
                slider(id::VOLUME, "Volume", a.volume as f32, 0.0, 150.0, 5.0, "%"),
                toggle(id::MUTE, "Mute", a.muted),
                list(
                    id::SINKS,
                    "Output devices",
                    sinks,
                    Some("USE"),
                    if a.available {
                        "no sinks"
                    } else {
                        "PipeWire not running"
                    },
                ),
            ],
        )
        .section(
            "Input",
            vec![
                toggle(id::MIC_MUTE, "Mute microphone", a.mic_muted),
                list(
                    id::SOURCES,
                    "Input devices",
                    sources,
                    Some("USE"),
                    "no sources",
                ),
                button(id::MIXER, "Advanced", "OPEN MIXER"),
            ],
        )
}

fn network(app: &App) -> Form {
    let n = &app.sys.network;
    let s = &app.scratch;
    let nets = n
        .networks
        .iter()
        .map(|w| {
            item(
                w.ssid.clone(),
                format!(
                    "{}%  {}",
                    w.signal,
                    if w.security.is_empty() {
                        "open"
                    } else {
                        &w.security
                    }
                ),
                w.in_use,
                if w.in_use { Some("connected") } else { None },
            )
        })
        .collect();
    let conns = n
        .connections
        .iter()
        .map(|c| {
            item(
                c.name.clone(),
                format!("{} {}", c.kind, c.device),
                c.active,
                if c.active { Some("active") } else { None },
            )
        })
        .collect();
    Form::default()
        .section(
            "Wi-Fi",
            vec![
                toggle(id::WIFI, "Wi-Fi", n.wifi_enabled),
                toggle(
                    id::AIRPLANE,
                    "Airplane mode",
                    !n.wifi_enabled && !n.available,
                ),
                button(id::RESCAN, "Networks", "RESCAN"),
                list(
                    id::NETWORKS,
                    "Available networks",
                    nets,
                    Some("CONNECT"),
                    if n.wifi_hardware {
                        "no networks found"
                    } else {
                        "no Wi-Fi hardware"
                    },
                ),
                secret(
                    id::PASSWORD,
                    "Password for new networks",
                    &s.wifi_password,
                    "leave empty for saved/open networks",
                ),
            ],
        )
        .section(
            "Connections",
            vec![
                info(
                    0,
                    "Connectivity",
                    format!(
                        "{}{}",
                        n.connectivity,
                        if n.ethernet_connected {
                            " (ethernet)"
                        } else {
                            ""
                        }
                    ),
                ),
                list(
                    id::CONNECTIONS,
                    "Saved connections",
                    conns,
                    Some("TOGGLE"),
                    "no connections",
                ),
                button(id::CONN_DELETE, "Selected connection", "DELETE"),
            ],
        )
        .section(
            "VPN",
            vec![
                text(
                    id::VPN_PATH,
                    "Import file",
                    &s.vpn_path,
                    "/path/to/config.ovpn or .conf",
                ),
                button(id::VPN_IMPORT, "Import OpenVPN / WireGuard", "IMPORT"),
                note(
                    0,
                    "Imported VPNs are managed from the Privacy panel (SUPER+P).",
                ),
            ],
        )
}

fn bluetooth(app: &App) -> Form {
    let b = &app.sys.bluetooth;
    let devs = b
        .devices
        .iter()
        .map(|d| {
            item(
                d.name.clone(),
                format!(
                    "{}{}",
                    d.mac,
                    d.battery.map(|p| format!("  {p}%")).unwrap_or_default()
                ),
                d.connected,
                if d.connected {
                    Some("connected")
                } else if d.paired {
                    Some("paired")
                } else {
                    None
                },
            )
        })
        .collect();
    Form::default()
        .section(
            "Adapter",
            vec![
                info(
                    0,
                    "Adapter",
                    if b.available {
                        b.adapter_name.clone()
                    } else {
                        "none".into()
                    },
                ),
                toggle(id::BT_POWER, "Bluetooth", b.powered),
                toggle(id::BT_SCAN, "Discover devices", b.discovering),
            ],
        )
        .section(
            "Devices",
            vec![
                list(
                    id::BT_DEVICES,
                    "Devices",
                    devs,
                    Some("TOGGLE"),
                    if b.powered {
                        "no devices"
                    } else {
                        "adapter off"
                    },
                ),
                button(id::BT_PAIR, "Selected device", "PAIR"),
                button(id::BT_REMOVE, "Selected device", "REMOVE"),
            ],
        )
}

const LID: [&str; 5] = ["suspend", "ignore", "lock", "poweroff", "hibernate"];

fn power(app: &App) -> Form {
    let c = &app.config;
    let p = &app.sys.power;
    let b = &app.sys.brightness;
    let profiles: Vec<String> = p.profiles.clone();
    let psel = profiles.iter().position(|x| *x == p.profile).unwrap_or(0);
    let battery = if p.battery_present {
        format!(
            "{:.0}% {}{}",
            p.battery_percent,
            p.battery_state,
            if p.time_to_empty_secs > 0 {
                format!(", {} min left", p.time_to_empty_secs / 60)
            } else {
                String::new()
            }
        )
    } else {
        "no battery".into()
    };
    let mut ctrls = vec![info(id::BATTERY, "Battery", battery)];
    if b.available {
        ctrls.push(slider(
            id::BRIGHTNESS,
            "Brightness",
            b.percent as f32,
            1.0,
            100.0,
            5.0,
            "%",
        ));
    }
    ctrls.push(choice_owned(id::PROFILE, "Power profile", profiles, psel));
    Form::default().section("Power", ctrls).section(
        "Idle",
        vec![
            slider(
                id::DIM,
                &format!("Dim screen after ({})", secs(c.power.dim_after)),
                c.power.dim_after as f32,
                0.0,
                3600.0,
                30.0,
                " s",
            ),
            slider(
                id::LOCK,
                &format!("Lock after ({})", secs(c.power.lock_after)),
                c.power.lock_after as f32,
                0.0,
                3600.0,
                30.0,
                " s",
            ),
            slider(
                id::DPMS,
                &format!("Screen off after ({})", secs(c.power.dpms_after)),
                c.power.dpms_after as f32,
                0.0,
                3600.0,
                30.0,
                " s",
            ),
            slider(
                id::SUSPEND,
                &format!("Suspend after ({})", secs(c.power.suspend_after)),
                c.power.suspend_after as f32,
                0.0,
                7200.0,
                60.0,
                " s",
            ),
            toggle(id::LOCK_SLEEP, "Lock before sleep", c.power.lock_on_sleep),
            choice(
                id::LID,
                "Lid close action",
                &LID,
                index_of(&LID, &c.power.lid_close),
            ),
        ],
    )
}

fn security(app: &App) -> Form {
    let f = &app.sys.fprint;
    let enrolled = f
        .enrolled
        .iter()
        .map(|e| item(e.clone(), "", false, None))
        .collect();
    let fingers: Vec<&str> = system::fprint::FINGERS.to_vec();
    let mut fp = vec![
        info(
            id::FP_DEVICE,
            "Reader",
            if f.available {
                f.device.clone()
            } else {
                "no fingerprint reader / fprintd".into()
            },
        ),
        list(id::FP_ENROLLED, "Enrolled fingers", enrolled, None, "none"),
        choice(
            id::FP_FINGER,
            "Finger to enrol",
            &fingers,
            app.scratch.finger,
        ),
        button(
            id::FP_ENROLL,
            "Enrol (touch the reader repeatedly)",
            "ENROL",
        ),
        button(id::FP_DELETE, "Remove all fingerprints", "DELETE"),
        toggle(
            id::FP_LOGIN,
            "Allow fingerprint at login/lock",
            app.config.privacy.fingerprint_login,
        ),
    ];
    if let Some(p) = &app.sys.fprint_progress {
        fp.insert(4, info(id::FP_PROGRESS, "Enrolment", p.clone()));
    }
    let keyring = app.sysmon.process_running("gnome-keyring-d");
    Form::default()
        .section("Screen lock", vec![
            button(id::LOCK_NOW, "hyprlock", "LOCK NOW"),
            note(0, "Lock timing lives under Power → Idle; the lock screen uses the eDEX theme from /usr/share/edex-de/hypr/hyprlock.conf."),
        ])
        .section("Fingerprint", fp)
        .section("System", vec![
            info(id::FIREWALL, "Firewall (nftables)", if app.sys.privacy.firewall_active { "active" } else { "inactive" }),
            info(id::KEYRING, "Keyring daemon", if keyring { "running" } else { "not running" }),
        ])
}

fn users(app: &App) -> Form {
    let s = &app.scratch;
    let cur = app.state.settings.form_state.list_cursor(id::USERS);
    let items = app
        .sys
        .users
        .iter()
        .enumerate()
        .map(|(i, u)| {
            item(
                u.name.clone(),
                format!(
                    "{}  uid {}{}",
                    u.real_name,
                    u.uid,
                    if u.locked { "  (locked)" } else { "" }
                ),
                i == cur,
                if u.admin { Some("admin") } else { None },
            )
        })
        .collect();
    let selected = app.sys.users.get(cur);
    Form::default()
        .section(
            "Accounts",
            vec![list(id::USERS, "Users", items, None, "no users")],
        )
        .section(
            format!("Edit {}", selected.map(|u| u.name.as_str()).unwrap_or("-")),
            vec![
                text(
                    id::REAL_NAME,
                    "Full name",
                    &s.real_name,
                    selected.map(|u| u.real_name.as_str()).unwrap_or(""),
                ),
                button(id::SET_NAME, "Full name", "SAVE"),
                button(id::PASSWD, "Password (opens in a terminal tab)", "CHANGE"),
                button(
                    id::LOCK_USER,
                    if selected.is_some_and(|u| u.locked) {
                        "Unlock account"
                    } else {
                        "Lock account"
                    },
                    "TOGGLE",
                ),
            ],
        )
}

const POSITIONS: [&str; 4] = ["top-right", "top-left", "bottom-right", "bottom-left"];

fn notifications(app: &App) -> Form {
    let c = &app.config.notifications;
    Form::default()
        .section(
            "Behaviour",
            vec![
                toggle(id::DND, "Do not disturb", c.dnd),
                slider(
                    id::TIMEOUT,
                    "Default timeout",
                    c.timeout_ms as f32,
                    1000.0,
                    30000.0,
                    500.0,
                    " ms",
                ),
                choice(
                    id::POSITION,
                    "Position",
                    &POSITIONS,
                    index_of(&POSITIONS, &c.position),
                ),
                slider(
                    id::MAX_VISIBLE,
                    "Max visible toasts",
                    c.max_visible as f32,
                    1.0,
                    10.0,
                    1.0,
                    "",
                ),
                text(
                    id::MUTED,
                    "Muted apps (comma separated)",
                    &c.muted_apps.join(", "),
                    "e.g. Discord, Spotify",
                ),
            ],
        )
        .section(
            "Actions",
            vec![
                button(id::TEST, "Send a test notification", "TEST"),
                button(id::CLEAR, "Clear history", "CLEAR"),
                info(
                    0,
                    "Server",
                    if app.notif_server.as_ref().is_some_and(|s| s.is_owner()) {
                        "edex-de owns org.freedesktop.Notifications"
                    } else {
                        "another notification daemon is running"
                    },
                ),
            ],
        )
}

fn services(app: &App) -> Form {
    let s = &app.scratch;
    let st = if s.services_user {
        &app.sys.services_user
    } else {
        &app.sys.services_system
    };
    let filter = s.service_filter.to_lowercase();
    let cur = app.state.settings.form_state.list_cursor(id::UNITS);
    let items = st
        .units
        .iter()
        .filter(|u| {
            filter.is_empty()
                || u.name.to_lowercase().contains(&filter)
                || u.description.to_lowercase().contains(&filter)
        })
        .take(200)
        .enumerate()
        .map(|(i, u)| {
            item(
                u.name.trim_end_matches(".service").to_string(),
                format!("{}  {}", u.active, u.description),
                i == cur,
                if u.enabled.is_empty() {
                    None
                } else {
                    Some(u.enabled.as_str())
                },
            )
        })
        .collect();
    Form::default().section(
        "systemd",
        vec![
            choice(
                id::SCOPE,
                "Scope",
                &["system", "user"],
                if s.services_user { 1 } else { 0 },
            ),
            text(id::FILTER, "Filter", &s.service_filter, "type to filter"),
            list(id::UNITS, "Units", items, Some("START/STOP"), "no units"),
            button(id::ENABLE, "Selected unit", "ENABLE"),
            button(id::DISABLE, "Selected unit", "DISABLE"),
            button(id::RESTART, "Selected unit", "RESTART"),
        ],
    )
}

const LAYOUTS: [&str; 3] = ["dwindle", "master", "scrolling"];

fn wm(app: &App) -> Form {
    let c = &app.config.wm;
    let binds = app
        .hypr
        .as_ref()
        .and_then(|h| h.binds().ok())
        .map(|b| {
            let mut lines: Vec<String> = b
                .iter()
                .filter(|x| !x.description.is_empty() || !x.dispatcher.is_empty())
                .take(60)
                .map(|x| {
                    format!(
                        "{}{}  →  {} {}",
                        modmask_name(x.modmask),
                        x.key,
                        x.dispatcher,
                        x.arg
                    )
                })
                .collect();
            if lines.is_empty() {
                lines.push("no binds reported".into());
            }
            lines.join("\n")
        })
        .unwrap_or_else(|| "Hyprland not connected".into());
    Form::default()
        .section(
            "Tiling",
            vec![
                slider(
                    id::GAPS_IN,
                    "Inner gaps",
                    c.gaps_in as f32,
                    0.0,
                    64.0,
                    1.0,
                    " px",
                ),
                slider(
                    id::GAPS_OUT,
                    "Outer gaps",
                    c.gaps_out as f32,
                    0.0,
                    128.0,
                    1.0,
                    " px",
                ),
                slider(
                    id::BORDER,
                    "Border width",
                    c.border as f32,
                    0.0,
                    10.0,
                    1.0,
                    " px",
                ),
                slider(
                    id::ROUNDING,
                    "Corner rounding",
                    c.rounding as f32,
                    0.0,
                    30.0,
                    1.0,
                    " px",
                ),
                choice(
                    id::LAYOUT,
                    "Layout",
                    &LAYOUTS,
                    index_of(&LAYOUTS, &c.layout),
                ),
                slider(
                    id::WORKSPACES,
                    "Workspaces",
                    c.workspaces as f32,
                    1.0,
                    10.0,
                    1.0,
                    "",
                ),
                toggle(id::WM_ANIM, "Window animations", c.animations),
                toggle(id::BLUR, "Blur", c.blur),
                button(
                    id::RELOAD,
                    "Regenerate and reload Hyprland config",
                    "RELOAD",
                ),
            ],
        )
        .section("Key bindings", vec![note(id::BINDS, &binds)])
}

fn modmask_name(mask: u32) -> String {
    let mut parts = Vec::new();
    if mask & 64 != 0 {
        parts.push("SUPER");
    }
    if mask & 4 != 0 {
        parts.push("CTRL");
    }
    if mask & 8 != 0 {
        parts.push("ALT");
    }
    if mask & 1 != 0 {
        parts.push("SHIFT");
    }
    if parts.is_empty() {
        String::new()
    } else {
        format!("{}+", parts.join("+"))
    }
}

const CURSORS: [&str; 3] = ["block", "underline", "beam"];
const BELLS: [&str; 3] = ["visual", "audible", "none"];

fn terminal(app: &App) -> Form {
    let c = &app.config.terminal;
    Form::default().section(
        "Terminal",
        vec![
            text(
                id::SHELL,
                "Shell (empty = login shell)",
                &c.shell,
                "/usr/bin/fish",
            ),
            slider(
                id::SCROLLBACK,
                "Scrollback lines",
                c.scrollback as f32,
                100.0,
                200000.0,
                1000.0,
                "",
            ),
            slider(
                id::TERM_FONT,
                "Font size",
                c.font_size,
                8.0,
                32.0,
                1.0,
                " px",
            ),
            choice(
                id::CURSOR,
                "Cursor",
                &CURSORS,
                index_of(&CURSORS, &c.cursor),
            ),
            toggle(id::BLINK, "Cursor blink", c.cursor_blink),
            choice(id::BELL, "Bell", &BELLS, index_of(&BELLS, &c.bell)),
            toggle(
                id::OSC52,
                "Allow programs to read the clipboard (OSC 52)",
                c.osc52_read,
            ),
        ],
    )
}

fn about(app: &App) -> Form {
    let a = &app.sys.about;
    let up = a.uptime_secs;
    Form::default().section(
        "About this system",
        vec![
            info(0, "OS", format!("{} {}", a.os_name, a.os_version)),
            info(0, "Kernel", a.kernel.clone()),
            info(0, "Hostname", a.hostname.clone()),
            info(0, "CPU", a.cpu.clone()),
            info(0, "GPU", a.gpu.clone()),
            info(
                0,
                "Memory",
                format!("{:.1} GiB", a.ram_total_kb as f64 / 1_048_576.0),
            ),
            info(0, "Hyprland", a.hyprland_version.clone()),
            info(0, "eDEX-DE", a.edex_version.clone()),
            info(
                0,
                "Uptime",
                format!(
                    "{}d {:02}h {:02}m",
                    up / 86400,
                    (up % 86400) / 3600,
                    (up % 3600) / 60
                ),
            ),
        ],
    )
}

// ─── Changes ────────────────────────────────────────────────────────────────

fn f_u32(c: &Change) -> Option<u32> {
    if let Change::Slider(v) = c {
        Some(v.round() as u32)
    } else {
        None
    }
}
fn f_bool(c: &Change) -> Option<bool> {
    if let Change::Toggle(v) = c {
        Some(*v)
    } else {
        None
    }
}
fn f_idx(c: &Change) -> Option<usize> {
    if let Change::Choice(i) = c {
        Some(*i)
    } else {
        None
    }
}
fn f_text(c: &Change) -> Option<String> {
    if let Change::Text(t) = c {
        Some(t.clone())
    } else {
        None
    }
}

pub fn on_change(app: &mut App, platform: &mut Platform<AppEvent>, id: u32, ch: Change) {
    let mut save = false;
    let mut hypr = false;
    {
        let c = &mut app.config;
        match id {
            id::THEME => {
                if let Some(i) = f_idx(&ch) {
                    if let Some(name) = app.themes.keys().nth(i) {
                        c.appearance.theme = name.clone();
                        save = true;
                    }
                }
            }
            id::FONT => {
                if let Some(t) = f_text(&ch) {
                    c.appearance.font = t;
                    save = true;
                }
            }
            id::FONT_SIZE => {
                if let Change::Slider(v) = ch {
                    c.appearance.font_size = v;
                    save = true;
                }
            }
            id::GLOW => {
                if let Change::Slider(v) = ch {
                    c.appearance.border_glow = v / 100.0;
                    save = true;
                }
            }
            id::SCANLINES => {
                if let Some(v) = f_bool(&ch) {
                    c.appearance.scanlines = v;
                    save = true;
                }
            }
            id::ANIMATIONS => {
                if let Some(v) = f_bool(&ch) {
                    c.appearance.animations = v;
                    save = true;
                }
            }
            id::BOOT => {
                if let Some(v) = f_bool(&ch) {
                    c.appearance.boot_animation = v;
                    save = true;
                }
            }
            id::THEME_APPS => {
                if let Some(v) = f_bool(&ch) {
                    c.appearance.theme_apps = v;
                    save = true;
                }
            }
            id::KEYBOARD => {
                if let Some(v) = f_bool(&ch) {
                    c.appearance.keyboard_visible = v;
                    save = true;
                }
            }
            id::SIDE_PANELS => {
                if let Some(v) = f_bool(&ch) {
                    c.layout.reserve_side_panels = v;
                    save = true;
                }
            }
            id::FS_SPLIT => {
                if let Change::Slider(v) = ch {
                    c.layout.fs_split = v / 100.0;
                    save = true;
                }
            }
            id::SYS_SPLIT => {
                if let Change::Slider(v) = ch {
                    c.layout.sysinfo_split = v / 100.0;
                    save = true;
                }
            }
            id::NIGHT => {
                if let Some(v) = f_bool(&ch) {
                    c.display.night_light = v;
                    save = true;
                    app.system.send(SysRequest::NightLight {
                        on: v,
                        temp: c.display.night_temp,
                    });
                }
            }
            id::NIGHT_TEMP => {
                if let Some(v) = f_u32(&ch) {
                    c.display.night_temp = v;
                    save = true;
                    if c.display.night_light {
                        app.system
                            .send(SysRequest::NightLight { on: true, temp: v });
                    }
                }
            }
            id::KB_LAYOUT => {
                match &ch {
                    Change::Choice(i) => {
                        if let Some(l) = app.sys.layouts.get(*i) {
                            c.input.kb_layout = l.code.clone();
                        }
                    }
                    Change::Text(t) => c.input.kb_layout = t.clone(),
                    _ => {}
                }
                save = true;
                hypr = true;
            }
            id::KB_VARIANT => {
                if let Some(t) = f_text(&ch) {
                    c.input.kb_variant = t;
                    save = true;
                    hypr = true;
                }
            }
            id::KB_OPTIONS => {
                if let Some(t) = f_text(&ch) {
                    c.input.kb_options = t;
                    save = true;
                    hypr = true;
                }
            }
            id::REPEAT_RATE => {
                if let Some(v) = f_u32(&ch) {
                    c.input.repeat_rate = v;
                    save = true;
                    hypr = true;
                }
            }
            id::REPEAT_DELAY => {
                if let Some(v) = f_u32(&ch) {
                    c.input.repeat_delay = v;
                    save = true;
                    hypr = true;
                }
            }
            id::NATURAL => {
                if let Some(v) = f_bool(&ch) {
                    c.input.natural_scroll = v;
                    save = true;
                    hypr = true;
                }
            }
            id::TAP => {
                if let Some(v) = f_bool(&ch) {
                    c.input.tap_to_click = v;
                    save = true;
                    hypr = true;
                }
            }
            id::SENS => {
                if let Change::Slider(v) = ch {
                    c.input.sensitivity = v;
                    save = true;
                    hypr = true;
                }
            }
            id::DIM => {
                if let Some(v) = f_u32(&ch) {
                    c.power.dim_after = v;
                    save = true;
                    hypr = true;
                }
            }
            id::LOCK => {
                if let Some(v) = f_u32(&ch) {
                    c.power.lock_after = v;
                    save = true;
                    hypr = true;
                }
            }
            id::DPMS => {
                if let Some(v) = f_u32(&ch) {
                    c.power.dpms_after = v;
                    save = true;
                    hypr = true;
                }
            }
            id::SUSPEND => {
                if let Some(v) = f_u32(&ch) {
                    c.power.suspend_after = v;
                    save = true;
                    hypr = true;
                }
            }
            id::LOCK_SLEEP => {
                if let Some(v) = f_bool(&ch) {
                    c.power.lock_on_sleep = v;
                    save = true;
                    hypr = true;
                }
            }
            id::FP_LOGIN => {
                if let Some(v) = f_bool(&ch) {
                    c.privacy.fingerprint_login = v;
                    save = true;
                }
            }
            id::DND => {
                if let Some(v) = f_bool(&ch) {
                    c.notifications.dnd = v;
                    save = true;
                }
            }
            id::TIMEOUT => {
                if let Some(v) = f_u32(&ch) {
                    c.notifications.timeout_ms = v;
                    save = true;
                }
            }
            id::POSITION => {
                if let Some(i) = f_idx(&ch) {
                    c.notifications.position = POSITIONS[i.min(3)].into();
                    save = true;
                }
            }
            id::MAX_VISIBLE => {
                if let Some(v) = f_u32(&ch) {
                    c.notifications.max_visible = v as usize;
                    save = true;
                }
            }
            id::MUTED => {
                if let Some(t) = f_text(&ch) {
                    c.notifications.muted_apps = t
                        .split(',')
                        .map(|s| s.trim().to_string())
                        .filter(|s| !s.is_empty())
                        .collect();
                    save = true;
                }
            }
            id::GAPS_IN => {
                if let Some(v) = f_u32(&ch) {
                    c.wm.gaps_in = v;
                    save = true;
                    hypr = true;
                }
            }
            id::GAPS_OUT => {
                if let Some(v) = f_u32(&ch) {
                    c.wm.gaps_out = v;
                    save = true;
                    hypr = true;
                }
            }
            id::BORDER => {
                if let Some(v) = f_u32(&ch) {
                    c.wm.border = v;
                    save = true;
                    hypr = true;
                }
            }
            id::ROUNDING => {
                if let Some(v) = f_u32(&ch) {
                    c.wm.rounding = v;
                    save = true;
                    hypr = true;
                }
            }
            id::LAYOUT => {
                if let Some(i) = f_idx(&ch) {
                    c.wm.layout = LAYOUTS[i.min(2)].into();
                    save = true;
                    hypr = true;
                }
            }
            id::WORKSPACES => {
                if let Some(v) = f_u32(&ch) {
                    c.wm.workspaces = v;
                    save = true;
                    hypr = true;
                }
            }
            id::WM_ANIM => {
                if let Some(v) = f_bool(&ch) {
                    c.wm.animations = v;
                    save = true;
                    hypr = true;
                }
            }
            id::BLUR => {
                if let Some(v) = f_bool(&ch) {
                    c.wm.blur = v;
                    save = true;
                    hypr = true;
                }
            }
            id::SHELL => {
                if let Some(t) = f_text(&ch) {
                    c.terminal.shell = t;
                    save = true;
                }
            }
            id::SCROLLBACK => {
                if let Some(v) = f_u32(&ch) {
                    c.terminal.scrollback = v as usize;
                    save = true;
                }
            }
            id::TERM_FONT => {
                if let Change::Slider(v) = ch {
                    c.terminal.font_size = v;
                    save = true;
                }
            }
            id::CURSOR => {
                if let Some(i) = f_idx(&ch) {
                    c.terminal.cursor = CURSORS[i.min(2)].into();
                    save = true;
                }
            }
            id::BLINK => {
                if let Some(v) = f_bool(&ch) {
                    c.terminal.cursor_blink = v;
                    save = true;
                }
            }
            id::BELL => {
                if let Some(i) = f_idx(&ch) {
                    c.terminal.bell = BELLS[i.min(2)].into();
                    save = true;
                }
            }
            id::OSC52 => {
                if let Some(v) = f_bool(&ch) {
                    c.terminal.osc52_read = v;
                    save = true;
                }
            }
            _ => {}
        }
    }
    if save {
        app.commit_config(platform, hypr);
        if id == id::THEME || id == id::FONT || id == id::FONT_SIZE || id == id::TERM_FONT {
            app.relayout(platform);
        }
        if hypr && (id::KB_LAYOUT..=id::SENS).contains(&id) {
            let c = &app.config.input;
            app.system.send(SysRequest::ApplyInput {
                kb_layout: c.kb_layout.clone(),
                kb_variant: c.kb_variant.clone(),
                kb_options: c.kb_options.clone(),
                repeat_rate: c.repeat_rate,
                repeat_delay: c.repeat_delay,
                natural_scroll: c.natural_scroll,
                tap_to_click: c.tap_to_click,
                sensitivity: c.sensitivity,
            });
        }
        return;
    }
    // Backend-driven controls.
    match id {
        // Display
        id::MONITORS => {
            if let Change::ListSelect(i) | Change::ListAction(i) = ch {
                app.scratch.monitor = i;
                if let Some(m) = app.sys.display.monitors.get(i) {
                    let saved = app
                        .config
                        .display
                        .monitors
                        .iter()
                        .find(|x| x.name == m.name);
                    app.scratch.mon_mode = 0;
                    app.scratch.mon_pos = saved
                        .map(|s| s.position.clone())
                        .unwrap_or_else(|| format!("{},{}", m.x, m.y));
                    app.scratch.mon_scale = m.scale;
                    app.scratch.mon_transform = m.transform.clamp(0, 7) as usize;
                    app.scratch.mon_disabled = m.disabled;
                }
            }
        }
        id::MON_MODE => {
            if let Some(i) = f_idx(&ch) {
                app.scratch.mon_mode = i;
            }
        }
        id::MON_POS => {
            if let Some(t) = f_text(&ch) {
                app.scratch.mon_pos = t;
            }
        }
        id::MON_SCALE => {
            if let Change::Slider(v) = ch {
                app.scratch.mon_scale = v;
            }
        }
        id::MON_TRANSFORM => {
            if let Some(i) = f_idx(&ch) {
                app.scratch.mon_transform = i;
            }
        }
        id::MON_DISABLED => {
            if let Some(v) = f_bool(&ch) {
                app.scratch.mon_disabled = v;
            }
        }
        id::MON_APPLY => {
            let s = &app.scratch;
            if let Some(m) = app.sys.display.monitors.get(s.monitor).cloned() {
                let mut modes = vec!["preferred".to_string(), "highrr".into(), "highres".into()];
                modes.extend(m.available_modes.iter().cloned());
                let mode = modes
                    .get(s.mon_mode)
                    .cloned()
                    .unwrap_or_else(|| "preferred".into());
                let position = if s.mon_pos.trim().is_empty() {
                    "auto".to_string()
                } else {
                    s.mon_pos.trim().to_string()
                };
                let entry = settings::Monitor {
                    name: m.name.clone(),
                    mode: mode.clone(),
                    position: position.clone(),
                    scale: s.mon_scale,
                    transform: s.mon_transform as u32,
                    disabled: s.mon_disabled,
                };
                app.system.send(SysRequest::ApplyMonitor {
                    name: m.name.clone(),
                    mode,
                    position,
                    scale: s.mon_scale,
                    transform: s.mon_transform as u32,
                    disabled: s.mon_disabled,
                });
                let mons = &mut app.config.display.monitors;
                if let Some(e) = mons.iter_mut().find(|x| x.name == m.name) {
                    *e = entry;
                } else {
                    mons.push(entry);
                }
                app.commit_config(platform, true);
                app.system.send(SysRequest::DisplayQuery {
                    night_light: app.config.display.night_light,
                    night_temp: app.config.display.night_temp,
                });
            }
        }
        // Audio
        id::VOLUME => {
            if let Some(v) = f_u32(&ch) {
                crate::status::audio_request(app, SysRequest::AudioSetVolume(v));
            }
        }
        id::MUTE => crate::status::audio_request(app, SysRequest::AudioToggleMute),
        id::MIC_MUTE => crate::status::audio_request(app, SysRequest::AudioToggleMicMute),
        id::SINKS => {
            if let Change::ListAction(i) = ch {
                if let Some(d) = app.sys.audio.sinks.get(i) {
                    app.system.send(SysRequest::AudioSetDefault(d.id));
                }
            }
        }
        id::SOURCES => {
            if let Change::ListAction(i) = ch {
                if let Some(d) = app.sys.audio.sources.get(i) {
                    app.system.send(SysRequest::AudioSetDefault(d.id));
                }
            }
        }
        id::MIXER => {
            let _ =
                launcher::runner::spawn_detached("pavucontrol || pwvucontrol", app.hypr.is_some());
        }
        // Network
        id::WIFI => {
            if let Some(v) = f_bool(&ch) {
                app.system.send(SysRequest::WifiRadio(v));
            }
        }
        id::AIRPLANE => {
            if let Some(v) = f_bool(&ch) {
                app.system.send(SysRequest::Airplane(v));
            }
        }
        id::RESCAN => app.system.send(SysRequest::NetworkQuery { rescan: true }),
        id::PASSWORD => {
            if let Some(t) = f_text(&ch) {
                app.scratch.wifi_password = t;
            }
        }
        id::NETWORKS => {
            if let Change::ListAction(i) = ch {
                if let Some(n) = app.sys.network.networks.get(i) {
                    let pw = app.scratch.wifi_password.clone();
                    app.state.settings.status = Some(format!("connecting to {}…", n.ssid));
                    app.system.send(SysRequest::WifiConnect {
                        ssid: n.ssid.clone(),
                        password: if pw.is_empty() { None } else { Some(pw) },
                    });
                    app.scratch.wifi_password.clear();
                }
            }
        }
        id::CONNECTIONS => {
            if let Change::ListAction(i) = ch {
                if let Some(c) = app.sys.network.connections.get(i) {
                    app.system.send(if c.active {
                        SysRequest::ConnectionDown(c.name.clone())
                    } else {
                        SysRequest::ConnectionUp(c.name.clone())
                    });
                }
            }
        }
        id::CONN_DELETE => {
            let cur = app.state.settings.form_state.list_cursor(id::CONNECTIONS);
            if let Some(c) = app.sys.network.connections.get(cur) {
                app.system
                    .send(SysRequest::ConnectionDelete(c.name.clone()));
            }
        }
        id::VPN_PATH => {
            if let Some(t) = f_text(&ch) {
                app.scratch.vpn_path = t;
            }
        }
        id::VPN_IMPORT => {
            let p = app.scratch.vpn_path.trim().to_string();
            if !p.is_empty() {
                app.system.send(SysRequest::VpnImport(p));
                app.scratch.vpn_path.clear();
            }
        }
        // Bluetooth
        id::BT_POWER => {
            if let Some(v) = f_bool(&ch) {
                app.system.send(SysRequest::BluetoothPower(v));
            }
        }
        id::BT_SCAN => {
            if let Some(v) = f_bool(&ch) {
                app.system.send(SysRequest::BluetoothScan(v));
            }
        }
        id::BT_DEVICES => {
            if let Change::ListAction(i) = ch {
                if let Some(d) = app.sys.bluetooth.devices.get(i) {
                    app.system.send(if d.connected {
                        SysRequest::BluetoothDisconnect(d.mac.clone())
                    } else if d.paired {
                        SysRequest::BluetoothConnect(d.mac.clone())
                    } else {
                        SysRequest::BluetoothPair(d.mac.clone())
                    });
                }
            }
        }
        id::BT_PAIR | id::BT_REMOVE => {
            let cur = app.state.settings.form_state.list_cursor(id::BT_DEVICES);
            if let Some(d) = app.sys.bluetooth.devices.get(cur) {
                app.system.send(if id == id::BT_PAIR {
                    SysRequest::BluetoothPair(d.mac.clone())
                } else {
                    SysRequest::BluetoothRemove(d.mac.clone())
                });
            }
        }
        // Power
        id::BRIGHTNESS => {
            if let Some(v) = f_u32(&ch) {
                app.system.send(SysRequest::BrightnessSet(v));
            }
        }
        id::PROFILE => {
            if let Some(i) = f_idx(&ch) {
                if let Some(p) = app.sys.power.profiles.get(i).cloned() {
                    app.config.power.profile = p.clone();
                    app.commit_config(platform, false);
                    app.system.send(SysRequest::SetPowerProfile(p));
                }
            }
        }
        id::LID => {
            if let Some(i) = f_idx(&ch) {
                app.config.power.lid_close = LID[i.min(4)].into();
                app.commit_config(platform, false);
                app.system
                    .send(SysRequest::SetLidAction(LID[i.min(4)].into()));
            }
        }
        // Security
        id::LOCK_NOW => {
            let _ = launcher::runner::spawn_detached("hyprlock", app.hypr.is_some());
        }
        id::FP_FINGER => {
            if let Some(i) = f_idx(&ch) {
                app.scratch.finger = i;
            }
        }
        id::FP_ENROLL => {
            let finger = system::fprint::FINGERS[app.scratch.finger.min(9)].to_string();
            app.sys.fprint_progress = Some("touch the reader…".into());
            app.system.send(SysRequest::FprintEnroll {
                user: app.state.username.clone(),
                finger,
            });
        }
        id::FP_DELETE => app.system.send(SysRequest::FprintDeleteAll {
            user: app.state.username.clone(),
        }),
        // Users
        id::USERS => {
            if let Change::ListSelect(i) | Change::ListAction(i) = ch {
                app.scratch.real_name = app
                    .sys
                    .users
                    .get(i)
                    .map(|u| u.real_name.clone())
                    .unwrap_or_default();
            }
        }
        id::REAL_NAME => {
            if let Some(t) = f_text(&ch) {
                app.scratch.real_name = t;
            }
        }
        id::SET_NAME => {
            let cur = app.state.settings.form_state.list_cursor(id::USERS);
            if let Some(u) = app.sys.users.get(cur) {
                app.system.send(SysRequest::SetRealName {
                    user: u.name.clone(),
                    name: app.scratch.real_name.clone(),
                });
            }
        }
        id::PASSWD => {
            let cur = app.state.settings.form_state.list_cursor(id::USERS);
            let user = app
                .sys
                .users
                .get(cur)
                .map(|u| u.name.clone())
                .unwrap_or_else(|| app.state.username.clone());
            app.close_overlay(platform);
            if let Err(e) = app.terminal.new_tab() {
                tracing::warn!("new tab: {e:#}");
            }
            let cmd = if user == app.state.username {
                "passwd\n".to_string()
            } else {
                format!("sudo passwd {user}\n")
            };
            app.terminal.write_input(cmd.as_bytes());
        }
        id::LOCK_USER => {
            let cur = app.state.settings.form_state.list_cursor(id::USERS);
            if let Some(u) = app.sys.users.get(cur) {
                app.system.send(SysRequest::SetUserLocked {
                    user: u.name.clone(),
                    locked: !u.locked,
                });
            }
        }
        // Notifications
        id::TEST => crate::status::push_local_notification(
            app,
            platform,
            "Test notification",
            "Notifications are working.",
        ),
        id::CLEAR => {
            app.store.clear_history();
            crate::status::sync_toasts(app);
        }
        // Services
        id::SCOPE => {
            if let Some(i) = f_idx(&ch) {
                app.scratch.services_user = i == 1;
                app.system.send(SysRequest::ServicesQuery {
                    user: app.scratch.services_user,
                });
            }
        }
        id::FILTER => {
            if let Some(t) = f_text(&ch) {
                app.scratch.service_filter = t;
            }
        }
        id::UNITS | id::ENABLE | id::DISABLE | id::RESTART => {
            let user = app.scratch.services_user;
            let st = if user {
                &app.sys.services_user
            } else {
                &app.sys.services_system
            };
            let filter = app.scratch.service_filter.to_lowercase();
            let cur = if let Change::ListAction(i) = ch {
                i
            } else {
                app.state.settings.form_state.list_cursor(id::UNITS)
            };
            let unit = st
                .units
                .iter()
                .filter(|u| {
                    filter.is_empty()
                        || u.name.to_lowercase().contains(&filter)
                        || u.description.to_lowercase().contains(&filter)
                })
                .nth(cur)
                .cloned();
            if let Some(u) = unit {
                let action = match id {
                    id::ENABLE => UnitAction::Enable,
                    id::DISABLE => UnitAction::Disable,
                    id::RESTART => UnitAction::Restart,
                    _ => {
                        if let Change::ListAction(_) = ch {
                            if u.active == "active" {
                                UnitAction::Stop
                            } else {
                                UnitAction::Start
                            }
                        } else {
                            return;
                        }
                    }
                };
                app.system.send(SysRequest::UnitAction {
                    user,
                    unit: u.name.clone(),
                    action,
                });
            }
        }
        // WM
        id::RELOAD => app.export_hypr(),
        _ => {}
    }
}
