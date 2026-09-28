//! NetworkManager via `nmcli` (terse, machine-readable output).

use anyhow::Result;

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
            };
            if c.kind == "vpn" || c.kind == "wireguard" {
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
