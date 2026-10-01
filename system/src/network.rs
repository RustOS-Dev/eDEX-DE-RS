//! Networking through the NetworkManager D-Bus API. On RustOS that API is served by
//! `rustos-nmd`; the calls used here are the subset listed in `docs/rustos-requirements.md`.
//!
//! The logic is written against the small [`Nm`] trait so it can be tested without a bus.

use std::collections::HashMap;

use anyhow::{anyhow, Context, Result};
use zbus::{
    blocking::{Connection as Bus, Proxy},
    zvariant::{ObjectPath, OwnedObjectPath, OwnedValue, Value},
};

#[derive(Clone, Debug, Default, PartialEq)]
pub struct WifiNetwork {
    pub ssid: String,
    pub signal: u8,
    pub security: String,
    pub in_use: bool,
    pub bssid: String,
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct Connection {
    pub name: String,
    pub kind: String,
    pub device: String,
    pub active: bool,
    /// WireGuard only: this tunnel's public key, to give to the server/peer.
    pub wg_public_key: String,
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct NetworkState {
    pub available: bool,
    pub connectivity: String,
    pub wifi_enabled: bool,
    pub wifi_hardware: bool,
    pub networks: Vec<WifiNetwork>,
    pub connections: Vec<Connection>,
    pub vpn_connections: Vec<Connection>,
    pub active_ssid: Option<String>,
    pub ethernet_connected: bool,
}

/// NetworkManager settings: `{setting: {key: value}}`.
pub type Settings = HashMap<String, HashMap<String, OwnedValue>>;

#[derive(Clone, Debug, Default, PartialEq)]
pub struct General {
    pub connectivity: u32,
    pub wifi_enabled: bool,
    pub wifi_hardware: bool,
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct SavedConnection {
    pub path: String,
    pub id: String,
    /// `connection.type`, e.g. `802-11-wireless`, `802-3-ethernet`, `wireguard`, `vpn`.
    pub kind: String,
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct ActiveConnection {
    pub path: String,
    /// Path of the saved connection it activates.
    pub connection: String,
    pub devices: Vec<String>,
}

pub const DEVICE_ETHERNET: u32 = 1;
pub const DEVICE_WIFI: u32 = 2;

#[derive(Clone, Debug, Default, PartialEq)]
pub struct Device {
    pub path: String,
    pub interface: String,
    pub kind: u32,
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct AccessPoint {
    pub path: String,
    pub ssid: String,
    pub strength: u8,
    pub bssid: String,
    /// `--`, `WPA`, `WPA2`, `WPA3`, `WEP`.
    pub security: String,
}

/// The NetworkManager operations eDEX-DE needs.
pub trait Nm {
    fn general(&self) -> Result<General>;
    fn connections(&self) -> Result<Vec<SavedConnection>>;
    fn active_connections(&self) -> Result<Vec<ActiveConnection>>;
    fn devices(&self) -> Result<Vec<Device>>;
    fn access_points(&self, device: &str, rescan: bool) -> Result<Vec<AccessPoint>>;
    fn active_access_point(&self, device: &str) -> Option<String>;
    fn activate(&self, connection: &str, device: &str) -> Result<()>;
    fn add_and_activate(&self, settings: Settings, device: &str, ap: &str) -> Result<()>;
    fn deactivate(&self, active: &str) -> Result<()>;
    fn delete(&self, connection: &str) -> Result<()>;
    fn set_wireless_enabled(&self, on: bool) -> Result<()>;
    fn add_connection(&self, settings: Settings) -> Result<String>;
    fn secrets(&self, connection: &str, setting: &str) -> Result<Settings>;
}

fn connectivity_name(c: u32) -> &'static str {
    match c {
        1 => "none",
        2 => "portal",
        3 => "limited",
        4 => "full",
        _ => "unknown",
    }
}

pub fn query_with(nm: &dyn Nm, rescan: bool) -> NetworkState {
    let mut st = NetworkState::default();
    let Ok(general) = nm.general() else {
        return st;
    };
    st.available = true;
    st.connectivity = connectivity_name(general.connectivity).into();
    st.wifi_enabled = general.wifi_enabled;
    st.wifi_hardware = general.wifi_hardware;

    let devices = nm.devices().unwrap_or_default();
    let iface = |path: &str| {
        devices
            .iter()
            .find(|d| d.path == path)
            .map(|d| d.interface.clone())
            .unwrap_or_default()
    };
    let active = nm.active_connections().unwrap_or_default();
    for c in nm.connections().unwrap_or_default() {
        let act = active.iter().find(|a| a.connection == c.path);
        let conn = Connection {
            name: c.id.clone(),
            kind: c.kind.clone(),
            device: act
                .and_then(|a| a.devices.first())
                .map(|d| iface(d))
                .unwrap_or_default(),
            active: act.is_some(),
            wg_public_key: String::new(),
        };
        if conn.kind == "vpn" || conn.kind == "wireguard" {
            let mut conn = conn;
            if conn.kind == "wireguard" {
                conn.wg_public_key = wireguard_public_key(nm, &c.path).unwrap_or_default();
            }
            st.vpn_connections.push(conn);
        } else {
            if conn.active && conn.kind.contains("ethernet") {
                st.ethernet_connected = true;
            }
            st.connections.push(conn);
        }
    }

    if st.wifi_enabled {
        for dev in devices.iter().filter(|d| d.kind == DEVICE_WIFI) {
            let current = nm.active_access_point(&dev.path);
            for ap in nm.access_points(&dev.path, rescan).unwrap_or_default() {
                if ap.ssid.is_empty() {
                    continue;
                }
                let net = WifiNetwork {
                    in_use: current.as_deref() == Some(ap.path.as_str()),
                    ssid: ap.ssid.clone(),
                    signal: ap.strength,
                    security: ap.security.clone(),
                    bssid: ap.bssid.clone(),
                };
                if net.in_use {
                    st.active_ssid = Some(net.ssid.clone());
                }
                match st.networks.iter_mut().find(|n| n.ssid == net.ssid) {
                    // Several access points of one network: keep the one in use, else the
                    // strongest.
                    Some(n) if !n.in_use && (net.in_use || net.signal > n.signal) => *n = net,
                    Some(_) => {}
                    None => st.networks.push(net),
                }
            }
        }
        st.networks
            .sort_by(|a, b| b.in_use.cmp(&a.in_use).then(b.signal.cmp(&a.signal)));
    }
    st
}

fn find_connection(nm: &dyn Nm, name: &str) -> Result<SavedConnection> {
    nm.connections()?
        .into_iter()
        .find(|c| c.id == name)
        .ok_or_else(|| anyhow!("no connection named {name}"))
}

fn str_value(s: &str) -> OwnedValue {
    OwnedValue::try_from(Value::from(s.to_string())).expect("strings convert")
}

/// Join a Wi-Fi network: reuse a saved profile with that SSID, else create one.
pub fn wifi_connect_with(nm: &dyn Nm, ssid: &str, password: Option<&str>) -> Result<()> {
    let devices = nm.devices()?;
    let dev = devices
        .iter()
        .find(|d| d.kind == DEVICE_WIFI)
        .ok_or_else(|| anyhow!("no Wi-Fi device"))?;
    if password.is_none() {
        if let Ok(saved) = find_connection(nm, ssid) {
            return nm.activate(&saved.path, &dev.path);
        }
    }
    let ap = nm
        .access_points(&dev.path, false)?
        .into_iter()
        .filter(|a| a.ssid == ssid)
        .max_by_key(|a| a.strength)
        .ok_or_else(|| anyhow!("{ssid} is not in range"))?;
    let mut settings: Settings = HashMap::new();
    settings.insert(
        "connection".into(),
        HashMap::from([
            ("id".into(), str_value(ssid)),
            ("type".into(), str_value("802-11-wireless")),
        ]),
    );
    if let Some(psk) = password {
        let key_mgmt = if ap.security.contains("WPA3") {
            "sae"
        } else {
            "wpa-psk"
        };
        settings.insert(
            "802-11-wireless-security".into(),
            HashMap::from([
                ("key-mgmt".into(), str_value(key_mgmt)),
                ("psk".into(), str_value(psk)),
            ]),
        );
    }
    nm.add_and_activate(settings, &dev.path, &ap.path)
}

pub fn connection_up_with(nm: &dyn Nm, name: &str) -> Result<()> {
    let c = find_connection(nm, name)?;
    // NetworkManager picks the device for "/" (any).
    nm.activate(&c.path, "/")
}

pub fn connection_down_with(nm: &dyn Nm, name: &str) -> Result<()> {
    let c = find_connection(nm, name)?;
    let active = nm
        .active_connections()?
        .into_iter()
        .find(|a| a.connection == c.path)
        .ok_or_else(|| anyhow!("{name} is not active"))?;
    nm.deactivate(&active.path)
}

pub fn connection_delete_with(nm: &dyn Nm, name: &str) -> Result<()> {
    let c = find_connection(nm, name)?;
    nm.delete(&c.path)
}

/// The public key of a WireGuard profile, derived from its private key with `wg pubkey`.
fn wireguard_public_key(nm: &dyn Nm, connection: &str) -> Option<String> {
    let secrets = nm.secrets(connection, "wireguard").ok()?;
    let private: String = secrets
        .get("wireguard")?
        .get("private-key")?
        .try_clone()
        .ok()
        .and_then(|v| String::try_from(v).ok())?;
    wg_pubkey(&private).ok()
}

fn wg_pubkey(private: &str) -> Result<String> {
    use std::io::Write;
    let mut child = std::process::Command::new("wg")
        .arg("pubkey")
        .stdin(std::process::Stdio::piped())
        .stdout(std::process::Stdio::piped())
        .spawn()
        .context("wg is not installed")?;
    child
        .stdin
        .take()
        .context("wg stdin")?
        .write_all(format!("{}\n", private.trim()).as_bytes())?;
    let out = child.wait_with_output()?;
    anyhow::ensure!(out.status.success(), "wg pubkey failed");
    Ok(String::from_utf8_lossy(&out.stdout).trim().to_string())
}

fn wg_genkey() -> Result<String> {
    let out = std::process::Command::new("wg")
        .arg("genkey")
        .output()
        .context("wg is not installed")?;
    anyhow::ensure!(out.status.success(), "wg genkey failed");
    Ok(String::from_utf8_lossy(&out.stdout).trim().to_string())
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct WireguardSpec {
    pub name: String,
    pub address: String,
    pub dns: String,
    pub peer_public_key: String,
    pub endpoint: String,
    pub allowed_ips: String,
    pub keepalive: u16,
}

impl WireguardSpec {
    pub fn validate(&self) -> Result<()> {
        let name_ok = !self.name.is_empty()
            && self.name.len() <= 15
            && self
                .name
                .chars()
                .all(|c| c.is_ascii_alphanumeric() || "_=+.-".contains(c));
        if !name_ok {
            return Err(anyhow!(
                "name: 1-15 letters, digits or _=+.- (it becomes the interface name)"
            ));
        }
        if self.address.trim().is_empty() || !self.address.contains('/') {
            return Err(anyhow!("address: e.g. 10.0.0.2/32"));
        }
        let key = self.peer_public_key.trim();
        if key.len() != 44 || !key.ends_with('=') {
            return Err(anyhow!(
                "peer public key: the 44-character base64 key from the server"
            ));
        }
        if !self.endpoint.contains(':') {
            return Err(anyhow!("endpoint: host:port, e.g. vpn.example.com:51820"));
        }
        Ok(())
    }

    pub fn to_conf(&self, private_key: &str) -> String {
        let mut c = format!(
            "[Interface]\nPrivateKey = {private_key}\nAddress = {}\n",
            self.address.trim()
        );
        if !self.dns.trim().is_empty() {
            c.push_str(&format!("DNS = {}\n", self.dns.trim()));
        }
        c.push_str(&format!(
            "\n[Peer]\nPublicKey = {}\nEndpoint = {}\nAllowedIPs = {}\n",
            self.peer_public_key.trim(),
            self.endpoint.trim(),
            self.allowed()
        ));
        if self.keepalive > 0 {
            c.push_str(&format!("PersistentKeepalive = {}\n", self.keepalive));
        }
        c
    }

    fn allowed(&self) -> &str {
        if self.allowed_ips.trim().is_empty() {
            "0.0.0.0/0, ::/0"
        } else {
            self.allowed_ips.trim()
        }
    }

    /// NetworkManager settings for this tunnel (not connected automatically).
    pub fn to_settings(&self, private_key: &str) -> Result<Settings> {
        let owned = |v: Value<'_>| OwnedValue::try_from(v).map_err(|e| anyhow!("{e}"));
        let list = |s: &str| -> Vec<String> {
            s.split([',', ' '])
                .map(str::trim)
                .filter(|x| !x.is_empty())
                .map(str::to_string)
                .collect()
        };
        let mut peer: HashMap<String, Value<'_>> = HashMap::new();
        peer.insert(
            "public-key".into(),
            Value::from(self.peer_public_key.trim()),
        );
        peer.insert("endpoint".into(), Value::from(self.endpoint.trim()));
        peer.insert("allowed-ips".into(), Value::from(list(self.allowed())));
        if self.keepalive > 0 {
            peer.insert(
                "persistent-keepalive".into(),
                Value::from(self.keepalive as u32),
            );
        }
        let (v4, v6): (Vec<String>, Vec<String>) = list(&self.address)
            .into_iter()
            .partition(|a| !a.contains(':'));
        let address_data = |addrs: &[String]| -> Vec<HashMap<String, Value<'static>>> {
            addrs
                .iter()
                .filter_map(|a| {
                    let (ip, prefix) = a.split_once('/')?;
                    Some(HashMap::from([
                        ("address".to_string(), Value::from(ip.to_string())),
                        (
                            "prefix".to_string(),
                            Value::from(prefix.parse::<u32>().ok()?),
                        ),
                    ]))
                })
                .collect()
        };
        let dns = list(&self.dns);
        let mut s: Settings = HashMap::new();
        s.insert(
            "connection".into(),
            HashMap::from([
                ("id".into(), owned(Value::from(self.name.as_str()))?),
                ("type".into(), owned(Value::from("wireguard"))?),
                (
                    "interface-name".into(),
                    owned(Value::from(self.name.as_str()))?,
                ),
                ("autoconnect".into(), owned(Value::from(false))?),
            ]),
        );
        s.insert(
            "wireguard".into(),
            HashMap::from([
                ("private-key".into(), owned(Value::from(private_key))?),
                ("peers".into(), owned(Value::from(vec![peer]))?),
            ]),
        );
        let mut ipv4 = HashMap::from([(
            "method".to_string(),
            owned(Value::from(if v4.is_empty() {
                "disabled"
            } else {
                "manual"
            }))?,
        )]);
        if !v4.is_empty() {
            ipv4.insert(
                "address-data".into(),
                owned(Value::from(address_data(&v4)))?,
            );
            let dns4: Vec<String> = dns.iter().filter(|d| !d.contains(':')).cloned().collect();
            if !dns4.is_empty() {
                ipv4.insert("dns-data".into(), owned(Value::from(dns4))?);
            }
        }
        s.insert("ipv4".into(), ipv4);
        let mut ipv6 = HashMap::from([(
            "method".to_string(),
            owned(Value::from(if v6.is_empty() {
                "disabled"
            } else {
                "manual"
            }))?,
        )]);
        if !v6.is_empty() {
            ipv6.insert(
                "address-data".into(),
                owned(Value::from(address_data(&v6)))?,
            );
        }
        s.insert("ipv6".into(), ipv6);
        Ok(s)
    }
}

/// Parse a `wg-quick` config (one peer) into a spec named `name`.
pub fn parse_wg_quick(name: &str, text: &str) -> Result<(WireguardSpec, String)> {
    let mut spec = WireguardSpec {
        name: name.to_string(),
        ..Default::default()
    };
    let mut private = String::new();
    let mut section = String::new();
    for line in text.lines() {
        let line = line.split('#').next().unwrap_or("").trim();
        if line.starts_with('[') {
            section = line.trim_matches(['[', ']']).to_ascii_lowercase();
            continue;
        }
        let Some((k, v)) = line.split_once('=') else {
            continue;
        };
        let (k, v) = (k.trim().to_ascii_lowercase(), v.trim().to_string());
        match (section.as_str(), k.as_str()) {
            ("interface", "privatekey") => private = v,
            ("interface", "address") => spec.address = v,
            ("interface", "dns") => spec.dns = v,
            ("peer", "publickey") => spec.peer_public_key = v,
            ("peer", "endpoint") => spec.endpoint = v,
            ("peer", "allowedips") => spec.allowed_ips = v,
            ("peer", "persistentkeepalive") => spec.keepalive = v.parse().unwrap_or(0),
            _ => {}
        }
    }
    anyhow::ensure!(
        !private.is_empty(),
        "no PrivateKey in the [Interface] section"
    );
    spec.validate()?;
    Ok((spec, private))
}

pub fn wireguard_create_with(nm: &dyn Nm, spec: &WireguardSpec) -> Result<String> {
    spec.validate()?;
    let private = wg_genkey()?;
    nm.add_connection(spec.to_settings(&private)?)?;
    wg_pubkey(&private)
}

pub fn vpn_import_with(nm: &dyn Nm, path: &str) -> Result<String> {
    if path.ends_with(".ovpn") {
        anyhow::bail!("OpenVPN is not available on RustOS; use a WireGuard config");
    }
    let text = std::fs::read_to_string(path).with_context(|| format!("reading {path}"))?;
    let name = std::path::Path::new(path)
        .file_stem()
        .map(|s| s.to_string_lossy().into_owned())
        .unwrap_or_else(|| "wg0".into());
    let (spec, private) = parse_wg_quick(&name, &text)?;
    nm.add_connection(spec.to_settings(&private)?)?;
    Ok(format!("Connection '{}' added", spec.name))
}

// ─── D-Bus ───────────────────────────────────────────────────────────────────

const NM: &str = "org.freedesktop.NetworkManager";
const NM_PATH: &str = "/org/freedesktop/NetworkManager";
const NM_SETTINGS_PATH: &str = "/org/freedesktop/NetworkManager/Settings";

/// NetworkManager on the system bus.
pub struct DbusNm {
    bus: Bus,
}

impl DbusNm {
    pub fn connect() -> Result<Self> {
        Ok(Self {
            bus: Bus::system().context("system bus")?,
        })
    }

    fn proxy(&self, path: &str, iface: &str) -> Result<Proxy<'_>> {
        Ok(Proxy::new(
            &self.bus,
            NM,
            path.to_string(),
            iface.to_string(),
        )?)
    }

    fn nm(&self) -> Result<Proxy<'_>> {
        self.proxy(NM_PATH, NM)
    }
}

fn path(p: &str) -> Result<ObjectPath<'_>> {
    ObjectPath::try_from(p).map_err(|e| anyhow!("bad object path {p}: {e}"))
}

fn paths(v: Vec<OwnedObjectPath>) -> Vec<String> {
    v.into_iter().map(|p| p.as_str().to_string()).collect()
}

/// `--`, `WPA2`, … from the access point flag words.
pub fn security_name(flags: u32, wpa: u32, rsn: u32) -> String {
    const KEY_MGMT_SAE: u32 = 0x400;
    if rsn & KEY_MGMT_SAE != 0 {
        "WPA3".into()
    } else if rsn != 0 {
        "WPA2".into()
    } else if wpa != 0 {
        "WPA".into()
    } else if flags & 0x1 != 0 {
        "WEP".into()
    } else {
        "--".into()
    }
}

impl Nm for DbusNm {
    fn general(&self) -> Result<General> {
        let nm = self.nm()?;
        Ok(General {
            connectivity: nm.get_property("Connectivity").unwrap_or(0),
            wifi_enabled: nm.get_property("WirelessEnabled")?,
            wifi_hardware: nm.get_property("WirelessHardwareEnabled").unwrap_or(false),
        })
    }

    fn connections(&self) -> Result<Vec<SavedConnection>> {
        let settings = self.proxy(NM_SETTINGS_PATH, "org.freedesktop.NetworkManager.Settings")?;
        let list: Vec<OwnedObjectPath> = settings.call("ListConnections", &())?;
        let mut out = Vec::new();
        for p in paths(list) {
            let c = self.proxy(&p, "org.freedesktop.NetworkManager.Settings.Connection")?;
            let s: Settings = c.call("GetSettings", &())?;
            let get = |key: &str| -> String {
                s.get("connection")
                    .and_then(|c| c.get(key))
                    .and_then(|v| v.try_clone().ok())
                    .and_then(|v| String::try_from(v).ok())
                    .unwrap_or_default()
            };
            out.push(SavedConnection {
                id: get("id"),
                kind: get("type"),
                path: p,
            });
        }
        Ok(out)
    }

    fn active_connections(&self) -> Result<Vec<ActiveConnection>> {
        let list: Vec<OwnedObjectPath> = self.nm()?.get_property("ActiveConnections")?;
        let mut out = Vec::new();
        for p in paths(list) {
            let a = self.proxy(&p, "org.freedesktop.NetworkManager.Connection.Active")?;
            let conn: OwnedObjectPath = a.get_property("Connection")?;
            let devices: Vec<OwnedObjectPath> = a.get_property("Devices").unwrap_or_default();
            out.push(ActiveConnection {
                path: p,
                connection: conn.as_str().to_string(),
                devices: paths(devices),
            });
        }
        Ok(out)
    }

    fn devices(&self) -> Result<Vec<Device>> {
        let list: Vec<OwnedObjectPath> = self.nm()?.call("GetDevices", &())?;
        let mut out = Vec::new();
        for p in paths(list) {
            let d = self.proxy(&p, "org.freedesktop.NetworkManager.Device")?;
            out.push(Device {
                interface: d.get_property("Interface").unwrap_or_default(),
                kind: d.get_property("DeviceType").unwrap_or(0),
                path: p,
            });
        }
        Ok(out)
    }

    fn access_points(&self, device: &str, rescan: bool) -> Result<Vec<AccessPoint>> {
        let w = self.proxy(device, "org.freedesktop.NetworkManager.Device.Wireless")?;
        if rescan {
            let opts: HashMap<String, OwnedValue> = HashMap::new();
            // A scan already in progress is not an error worth reporting.
            let _ = w.call::<_, _, ()>("RequestScan", &(opts,));
            std::thread::sleep(std::time::Duration::from_secs(2));
        }
        let list: Vec<OwnedObjectPath> = w.call("GetAllAccessPoints", &())?;
        let mut out = Vec::new();
        for p in paths(list) {
            let ap = self.proxy(&p, "org.freedesktop.NetworkManager.AccessPoint")?;
            let ssid: Vec<u8> = ap.get_property("Ssid").unwrap_or_default();
            out.push(AccessPoint {
                ssid: String::from_utf8_lossy(&ssid).into_owned(),
                strength: ap.get_property("Strength").unwrap_or(0),
                bssid: ap.get_property("HwAddress").unwrap_or_default(),
                security: security_name(
                    ap.get_property("Flags").unwrap_or(0),
                    ap.get_property("WpaFlags").unwrap_or(0),
                    ap.get_property("RsnFlags").unwrap_or(0),
                ),
                path: p,
            });
        }
        Ok(out)
    }

    fn active_access_point(&self, device: &str) -> Option<String> {
        let w = self
            .proxy(device, "org.freedesktop.NetworkManager.Device.Wireless")
            .ok()?;
        let p: OwnedObjectPath = w.get_property("ActiveAccessPoint").ok()?;
        (p.as_str() != "/").then(|| p.as_str().to_string())
    }

    fn activate(&self, connection: &str, device: &str) -> Result<()> {
        let _: OwnedObjectPath = self.nm()?.call(
            "ActivateConnection",
            &(path(connection)?, path(device)?, path("/")?),
        )?;
        Ok(())
    }

    fn add_and_activate(&self, settings: Settings, device: &str, ap: &str) -> Result<()> {
        let _: (OwnedObjectPath, OwnedObjectPath) = self.nm()?.call(
            "AddAndActivateConnection",
            &(settings, path(device)?, path(ap)?),
        )?;
        Ok(())
    }

    fn deactivate(&self, active: &str) -> Result<()> {
        self.nm()?
            .call::<_, _, ()>("DeactivateConnection", &(path(active)?,))?;
        Ok(())
    }

    fn delete(&self, connection: &str) -> Result<()> {
        self.proxy(
            connection,
            "org.freedesktop.NetworkManager.Settings.Connection",
        )?
        .call::<_, _, ()>("Delete", &())?;
        Ok(())
    }

    fn set_wireless_enabled(&self, on: bool) -> Result<()> {
        self.nm()?.set_property("WirelessEnabled", on)?;
        Ok(())
    }

    fn add_connection(&self, settings: Settings) -> Result<String> {
        let p: OwnedObjectPath = self
            .proxy(NM_SETTINGS_PATH, "org.freedesktop.NetworkManager.Settings")?
            .call("AddConnection", &(settings,))?;
        Ok(p.as_str().to_string())
    }

    fn secrets(&self, connection: &str, setting: &str) -> Result<Settings> {
        Ok(self
            .proxy(
                connection,
                "org.freedesktop.NetworkManager.Settings.Connection",
            )?
            .call("GetSecrets", &(setting,))?)
    }
}

fn bus() -> Result<DbusNm> {
    DbusNm::connect().context("NetworkManager (rustos-nmd) is not running")
}

pub fn query(rescan: bool) -> NetworkState {
    match DbusNm::connect() {
        Ok(nm) => query_with(&nm, rescan),
        Err(_) => NetworkState::default(),
    }
}

pub fn wifi_connect(ssid: &str, password: Option<&str>) -> Result<()> {
    wifi_connect_with(&bus()?, ssid, password)
}

pub fn connection_up(name: &str) -> Result<()> {
    connection_up_with(&bus()?, name)
}

pub fn connection_down(name: &str) -> Result<()> {
    connection_down_with(&bus()?, name)
}

pub fn connection_delete(name: &str) -> Result<()> {
    connection_delete_with(&bus()?, name)
}

pub fn wifi_radio(on: bool) -> Result<()> {
    bus()?.set_wireless_enabled(on)
}

/// Airplane mode: Wi-Fi and Bluetooth radios off (RustOS has no rfkill).
pub fn airplane(on: bool) -> Result<()> {
    let wifi = wifi_radio(!on);
    let bt = crate::bluetooth::power(!on);
    wifi.and(bt)
}

pub fn wireguard_create(spec: &WireguardSpec) -> Result<String> {
    wireguard_create_with(&bus()?, spec)
}

pub fn vpn_import(path: &str) -> Result<String> {
    vpn_import_with(&bus()?, path)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::cell::RefCell;

    #[derive(Default)]
    struct FakeNm {
        calls: RefCell<Vec<String>>,
        added: RefCell<Vec<Settings>>,
    }

    impl Nm for FakeNm {
        fn general(&self) -> Result<General> {
            Ok(General {
                connectivity: 4,
                wifi_enabled: true,
                wifi_hardware: true,
            })
        }
        fn connections(&self) -> Result<Vec<SavedConnection>> {
            Ok(vec![
                SavedConnection {
                    path: "/s/1".into(),
                    id: "Home:5G".into(),
                    kind: "802-11-wireless".into(),
                },
                SavedConnection {
                    path: "/s/2".into(),
                    id: "Wired".into(),
                    kind: "802-3-ethernet".into(),
                },
                SavedConnection {
                    path: "/s/3".into(),
                    id: "wg0".into(),
                    kind: "wireguard".into(),
                },
            ])
        }
        fn active_connections(&self) -> Result<Vec<ActiveConnection>> {
            Ok(vec![
                ActiveConnection {
                    path: "/a/1".into(),
                    connection: "/s/1".into(),
                    devices: vec!["/d/wlan".into()],
                },
                ActiveConnection {
                    path: "/a/2".into(),
                    connection: "/s/2".into(),
                    devices: vec!["/d/eth".into()],
                },
            ])
        }
        fn devices(&self) -> Result<Vec<Device>> {
            Ok(vec![
                Device {
                    path: "/d/wlan".into(),
                    interface: "wlan0".into(),
                    kind: DEVICE_WIFI,
                },
                Device {
                    path: "/d/eth".into(),
                    interface: "eth0".into(),
                    kind: DEVICE_ETHERNET,
                },
            ])
        }
        fn access_points(&self, _device: &str, _rescan: bool) -> Result<Vec<AccessPoint>> {
            let ap = |path: &str, ssid: &str, strength, security: &str| AccessPoint {
                path: path.into(),
                ssid: ssid.into(),
                strength,
                bssid: format!("{path}-mac"),
                security: security.into(),
            };
            Ok(vec![
                ap("/ap/1", "Home:5G", 88, "WPA2"),
                ap("/ap/2", "Cafe", 40, "--"),
                ap("/ap/3", "Home:5G", 95, "WPA2"),
                ap("/ap/4", "", 10, "--"),
                ap("/ap/5", "Lab", 70, "WPA3"),
            ])
        }
        fn active_access_point(&self, _device: &str) -> Option<String> {
            Some("/ap/1".into())
        }
        fn activate(&self, connection: &str, device: &str) -> Result<()> {
            self.calls
                .borrow_mut()
                .push(format!("activate {connection} {device}"));
            Ok(())
        }
        fn add_and_activate(&self, settings: Settings, device: &str, ap: &str) -> Result<()> {
            self.calls
                .borrow_mut()
                .push(format!("add-activate {device} {ap}"));
            self.added.borrow_mut().push(settings);
            Ok(())
        }
        fn deactivate(&self, active: &str) -> Result<()> {
            self.calls.borrow_mut().push(format!("deactivate {active}"));
            Ok(())
        }
        fn delete(&self, connection: &str) -> Result<()> {
            self.calls.borrow_mut().push(format!("delete {connection}"));
            Ok(())
        }
        fn set_wireless_enabled(&self, on: bool) -> Result<()> {
            self.calls.borrow_mut().push(format!("wifi {on}"));
            Ok(())
        }
        fn add_connection(&self, settings: Settings) -> Result<String> {
            self.added.borrow_mut().push(settings);
            Ok("/s/9".into())
        }
        fn secrets(&self, _connection: &str, _setting: &str) -> Result<Settings> {
            Err(anyhow!("no secrets in tests"))
        }
    }

    fn spec() -> WireguardSpec {
        WireguardSpec {
            name: "wg-home".into(),
            address: "10.8.0.2/32".into(),
            dns: "10.8.0.1".into(),
            peer_public_key: "xTIBA5rboUvnH4htodjb6e697QjLERt1NAB4mZqp8Dg=".into(),
            endpoint: "vpn.example.com:51820".into(),
            allowed_ips: String::new(),
            keepalive: 25,
        }
    }

    #[test]
    fn builds_state_from_the_bus() {
        let nm = FakeNm::default();
        let s = query_with(&nm, false);
        assert!(s.available && s.wifi_enabled && s.ethernet_connected);
        assert_eq!(s.connectivity, "full");
        assert_eq!(s.active_ssid.as_deref(), Some("Home:5G"));
        // Duplicate SSIDs collapse to the one in use; hidden networks are skipped.
        assert_eq!(s.networks.len(), 3);
        assert_eq!(s.networks[0].bssid, "/ap/1-mac");
        assert_eq!(s.connections.len(), 2);
        assert_eq!(s.connections[0].device, "wlan0");
        assert_eq!(s.vpn_connections.len(), 1);
        assert!(!s.vpn_connections[0].active);
    }

    #[test]
    fn connects_and_controls_connections() {
        let nm = FakeNm::default();
        wifi_connect_with(&nm, "Home:5G", None).unwrap();
        wifi_connect_with(&nm, "Lab", Some("secret")).unwrap();
        connection_down_with(&nm, "Wired").unwrap();
        connection_up_with(&nm, "Wired").unwrap();
        connection_delete_with(&nm, "wg0").unwrap();
        assert!(connection_down_with(&nm, "wg0").is_err());
        assert_eq!(
            *nm.calls.borrow(),
            [
                "activate /s/1 /d/wlan",
                "add-activate /d/wlan /ap/5",
                "deactivate /a/2",
                "activate /s/2 /",
                "delete /s/3",
            ]
        );
        let added = nm.added.borrow();
        let sec = &added[0]["802-11-wireless-security"];
        assert_eq!(
            String::try_from(sec["key-mgmt"].try_clone().unwrap()).unwrap(),
            "sae"
        );
        assert!(wifi_connect_with(&nm, "Nowhere", Some("x")).is_err());
    }

    #[test]
    fn wireguard_spec_validation_conf_and_settings() {
        assert!(spec().validate().is_ok());
        let bad = |f: fn(&mut WireguardSpec)| {
            let mut s = spec();
            f(&mut s);
            s.validate().is_err()
        };
        assert!(bad(|s| s.name = "this-name-is-too-long".into()));
        assert!(bad(|s| s.name = "has space".into()));
        assert!(bad(|s| s.address = "10.8.0.2".into()));
        assert!(bad(|s| s.peer_public_key = "short".into()));
        assert!(bad(|s| s.endpoint = "vpn.example.com".into()));
        let conf = spec().to_conf("PRIVATEKEY=");
        assert!(conf.contains("PrivateKey = PRIVATEKEY=\nAddress = 10.8.0.2/32\nDNS = 10.8.0.1\n"));
        assert!(conf.contains("AllowedIPs = 0.0.0.0/0, ::/0\n"));
        let s = spec().to_settings("PRIVATEKEY=").unwrap();
        assert_eq!(
            String::try_from(s["connection"]["type"].try_clone().unwrap()).unwrap(),
            "wireguard"
        );
        assert_eq!(
            String::try_from(s["ipv4"]["method"].try_clone().unwrap()).unwrap(),
            "manual"
        );
        assert_eq!(
            String::try_from(s["ipv6"]["method"].try_clone().unwrap()).unwrap(),
            "disabled"
        );
        // A wg-quick file round-trips.
        let (parsed, private) = parse_wg_quick("wg-home", &conf).unwrap();
        assert_eq!(private, "PRIVATEKEY=");
        assert_eq!(parsed.endpoint, spec().endpoint);
        assert_eq!(parsed.keepalive, 25);
        assert!(parse_wg_quick("x", "[Peer]\nPublicKey = a\n").is_err());
    }

    #[test]
    fn imports_wireguard_but_not_openvpn() {
        let nm = FakeNm::default();
        let dir = tempfile::tempdir().unwrap();
        let p = dir.path().join("office.conf");
        std::fs::write(
            &p,
            spec().to_conf("PRIVATEKEY=").replace("wg-home", "office"),
        )
        .unwrap();
        let msg = vpn_import_with(&nm, p.to_str().unwrap()).unwrap();
        assert!(msg.contains("office"));
        assert_eq!(nm.added.borrow().len(), 1);
        assert!(vpn_import_with(&nm, "/x/work.ovpn").is_err());
    }

    #[test]
    fn names_security() {
        assert_eq!(security_name(0, 0, 0), "--");
        assert_eq!(security_name(1, 0, 0), "WEP");
        assert_eq!(security_name(1, 0x100, 0), "WPA");
        assert_eq!(security_name(1, 0, 0x100), "WPA2");
        assert_eq!(security_name(1, 0, 0x500), "WPA3");
    }
}
