//! NetworkManager via `nmcli` (terse, machine-readable output).

use anyhow::{anyhow, Result};

use crate::runner::CommandRunner;

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

/// nmcli terse output escapes `:` as `\:`; split on unescaped colons.
pub fn split_terse(line: &str) -> Vec<String> {
    let mut out = Vec::new();
    let mut cur = String::new();
    let mut chars = line.chars().peekable();
    while let Some(c) = chars.next() {
        match c {
            '\\' => {
                if let Some(n) = chars.next() {
                    cur.push(n);
                }
            }
            ':' => {
                out.push(std::mem::take(&mut cur));
            }
            _ => cur.push(c),
        }
    }
    out.push(cur);
    out
}

pub fn query(r: &dyn CommandRunner, rescan: bool) -> NetworkState {
    let mut st = NetworkState::default();
    let Ok(general) = r.run(
        "nmcli",
        &["-t", "-f", "STATE,CONNECTIVITY,WIFI-HW,WIFI", "general"],
    ) else {
        return st;
    };
    if !general.ok() {
        return st;
    }
    st.available = true;
    let f = split_terse(general.stdout.trim());
    st.connectivity = f.get(1).cloned().unwrap_or_default();
    st.wifi_hardware = f.get(2).map(|s| s == "enabled").unwrap_or(false);
    st.wifi_enabled = f.get(3).map(|s| s == "enabled").unwrap_or(false);

    if let Ok(out) = r.run(
        "nmcli",
        &["-t", "-f", "NAME,TYPE,DEVICE,ACTIVE", "connection", "show"],
    ) {
        for line in out.stdout.lines() {
            let f = split_terse(line);
            if f.len() < 4 {
                continue;
            }
            let c = Connection {
                name: f[0].clone(),
                kind: f[1].clone(),
                device: f[2].clone(),
                active: f[3] == "yes",
                wg_public_key: String::new(),
            };
            if c.kind == "vpn" || c.kind == "wireguard" {
                let mut c = c;
                if c.kind == "wireguard" {
                    c.wg_public_key = wireguard_public_key(r, &c.name).unwrap_or_default();
                }
                st.vpn_connections.push(c);
            } else {
                if c.active && c.kind.contains("ethernet") {
                    st.ethernet_connected = true;
                }
                st.connections.push(c);
            }
        }
    }
    if st.wifi_enabled {
        let mut args = vec![
            "-t",
            "-f",
            "IN-USE,SSID,SIGNAL,SECURITY,BSSID",
            "device",
            "wifi",
            "list",
        ];
        if rescan {
            args.extend(["--rescan", "yes"]);
        }
        if let Ok(out) = r.run("nmcli", &args) {
            for line in out.stdout.lines() {
                let f = split_terse(line);
                if f.len() < 5 || f[1].is_empty() {
                    continue;
                }
                let net = WifiNetwork {
                    in_use: f[0] == "*",
                    ssid: f[1].clone(),
                    signal: f[2].parse().unwrap_or(0),
                    security: f[3].clone(),
                    bssid: f[4].clone(),
                };
                if net.in_use {
                    st.active_ssid = Some(net.ssid.clone());
                }
                if !st.networks.iter().any(|n| n.ssid == net.ssid) {
                    st.networks.push(net);
                }
            }
            st.networks
                .sort_by(|a, b| b.in_use.cmp(&a.in_use).then(b.signal.cmp(&a.signal)));
        }
    }
    st
}

pub fn wifi_connect(r: &dyn CommandRunner, ssid: &str, password: Option<&str>) -> Result<()> {
    let mut args = vec!["device", "wifi", "connect", ssid];
    if let Some(p) = password {
        args.extend(["password", p]);
    }
    r.run_ok("nmcli", &args).map(|_| ())
}

pub fn connection_up(r: &dyn CommandRunner, name: &str) -> Result<()> {
    r.run_ok("nmcli", &["connection", "up", "id", name])
        .map(|_| ())
}

pub fn connection_down(r: &dyn CommandRunner, name: &str) -> Result<()> {
    r.run_ok("nmcli", &["connection", "down", "id", name])
        .map(|_| ())
}

pub fn connection_delete(r: &dyn CommandRunner, name: &str) -> Result<()> {
    r.run_ok("nmcli", &["connection", "delete", "id", name])
        .map(|_| ())
}

pub fn wifi_radio(r: &dyn CommandRunner, on: bool) -> Result<()> {
    r.run_ok("nmcli", &["radio", "wifi", if on { "on" } else { "off" }])
        .map(|_| ())
}

pub fn airplane(r: &dyn CommandRunner, on: bool) -> Result<()> {
    r.run_ok("rfkill", &[if on { "block" } else { "unblock" }, "all"])
        .map(|_| ())
}

/// Public key of a WireGuard connection (derived from its private key, which stays in NM).
pub fn wireguard_public_key(r: &dyn CommandRunner, name: &str) -> Option<String> {
    let private = r
        .run_ok(
            "nmcli",
            &[
                "-s",
                "-g",
                "wireguard.private-key",
                "connection",
                "show",
                name,
            ],
        )
        .ok()?;
    let private = private.trim();
    if private.is_empty() {
        return None;
    }
    let out = r
        .run_with_stdin("wg", &["pubkey"], &format!("{private}\n"))
        .ok()?;
    out.ok().then(|| out.stdout.trim().to_string())
}

/// What the user fills in to create a WireGuard tunnel; the key pair is generated here.
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
    /// The wg-quick style config NetworkManager imports.
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
        let allowed = if self.allowed_ips.trim().is_empty() {
            "0.0.0.0/0, ::/0"
        } else {
            self.allowed_ips.trim()
        };
        c.push_str(&format!(
            "\n[Peer]\nPublicKey = {}\nEndpoint = {}\nAllowedIPs = {allowed}\n",
            self.peer_public_key.trim(),
            self.endpoint.trim()
        ));
        if self.keepalive > 0 {
            c.push_str(&format!("PersistentKeepalive = {}\n", self.keepalive));
        }
        c
    }
}

/// Create a WireGuard tunnel in NetworkManager with a freshly generated key pair. Returns the
/// tunnel's public key for the server side.
pub fn wireguard_create(r: &dyn CommandRunner, spec: &WireguardSpec) -> Result<String> {
    spec.validate()?;
    let private = r.run_ok("wg", &["genkey"])?.trim().to_string();
    let dir = std::env::var_os("XDG_RUNTIME_DIR")
        .map(std::path::PathBuf::from)
        .unwrap_or_else(std::env::temp_dir)
        .join(format!("edex-wg-{}", std::process::id()));
    std::fs::create_dir_all(&dir)?;
    // NetworkManager names the connection (and interface) after the file.
    let path = dir.join(format!("{}.conf", spec.name));
    {
        use std::{io::Write, os::unix::fs::OpenOptionsExt};
        let mut f = std::fs::OpenOptions::new()
            .write(true)
            .create(true)
            .truncate(true)
            .mode(0o600)
            .open(&path)?;
        f.write_all(spec.to_conf(&private).as_bytes())?;
    }
    let path_str = path.to_string_lossy().to_string();
    let imported = r.run_ok(
        "nmcli",
        &[
            "connection",
            "import",
            "type",
            "wireguard",
            "file",
            &path_str,
        ],
    );
    let _ = std::fs::remove_dir_all(&dir);
    imported?;
    // Connect on request only: importing activates the connection right away, which with
    // AllowedIPs 0.0.0.0/0 would send all traffic to a server that may not know this key yet.
    let _ = r.run(
        "nmcli",
        &[
            "connection",
            "modify",
            &spec.name,
            "connection.autoconnect",
            "no",
        ],
    );
    let _ = r.run("nmcli", &["connection", "down", &spec.name]);
    let out = r.run_with_stdin("wg", &["pubkey"], &format!("{private}\n"))?;
    Ok(out.stdout.trim().to_string())
}

/// Import a WireGuard (`.conf`) or OpenVPN (`.ovpn`) profile.
pub fn vpn_import(r: &dyn CommandRunner, path: &str) -> Result<String> {
    let kind = if path.ends_with(".ovpn") {
        "openvpn"
    } else {
        "wireguard"
    };
    let out = r.run_ok(
        "nmcli",
        &["connection", "import", "type", kind, "file", path],
    )?;
    Ok(out.trim().to_string())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::runner::FakeRunner;

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
    fn wireguard_spec_validation_and_conf() {
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
        assert!(conf.contains("PersistentKeepalive = 25\n"));
    }

    #[test]
    fn wireguard_create_imports_and_returns_public_key() {
        let r = FakeRunner::default()
            .with("wg genkey", "PRIVATEKEY=\n")
            .with("wg pubkey", "PUBLICKEY=\n")
            .with("nmcli", "Connection 'wg-home' successfully added.\n");
        let public = wireguard_create(&r, &spec()).unwrap();
        assert_eq!(public, "PUBLICKEY=");
        let calls = r.calls();
        assert!(
            calls.iter().any(
                |c| c.starts_with("nmcli connection import type wireguard file ")
                    && c.ends_with("/wg-home.conf")
            ),
            "{calls:?}"
        );
        assert!(calls
            .contains(&"nmcli connection modify wg-home connection.autoconnect no".to_string()));
        assert!(calls.contains(&"nmcli connection down wg-home".to_string()));
    }

    #[test]
    fn parses_nmcli() {
        let r = FakeRunner::default()
            .with("nmcli -t -f STATE,CONNECTIVITY,WIFI-HW,WIFI general", "connected:full:enabled:enabled\n")
            .with("nmcli -t -f NAME,TYPE,DEVICE,ACTIVE connection show", "Home\\:5G:802-11-wireless:wlan0:yes\nWork VPN:vpn::no\nwg0:wireguard::no\nWired:802-3-ethernet:eth0:yes\n")
            .with("nmcli -t -f IN-USE,SSID,SIGNAL,SECURITY,BSSID device wifi list", "*:Home\\:5G:88:WPA2:AA\\:BB\n :Cafe:40:--:CC\\:DD\n :Home\\:5G:70:WPA2:EE\\:FF\n");
        let s = query(&r, false);
        assert!(s.available && s.wifi_enabled && s.ethernet_connected);
        assert_eq!(s.active_ssid.as_deref(), Some("Home:5G"));
        assert_eq!(s.networks.len(), 2);
        assert_eq!(s.networks[0].bssid, "AA:BB");
        assert_eq!(s.vpn_connections.len(), 2);
        assert_eq!(s.connections.len(), 2);
    }
}
