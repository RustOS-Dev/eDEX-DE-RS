//! Networking on RustOS: `ip -br addr` for the interfaces, `wifi` (its own drivers, or
//! wpa_supplicant for Linux-driver adapters) for wireless; saved networks are wifi.conf.

use std::{
    sync::Mutex,
    time::{Duration, Instant},
};

use anyhow::{anyhow, Result};

use crate::{
    network::{Connection, NetworkState, WifiNetwork},
    runner::CommandRunner,
};

/// An interface from `ip -br addr`: "NAME STATE ADDR...".
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Iface {
    pub name: String,
    pub up: bool,
    pub carrier: bool,
    pub addrs: Vec<String>,
}

pub fn parse_ip_brief(text: &str) -> Vec<Iface> {
    text.lines()
        .filter_map(|l| {
            let mut w = l.split_whitespace();
            let name = w.next()?.to_string();
            let state = w.next()?;
            Some(Iface {
                name,
                up: state != "DOWN",
                carrier: state == "UP",
                addrs: w.map(String::from).collect(),
            })
        })
        .collect()
}

/// `wifi status`: "wlan0: connected" then "  key value" lines.
pub fn parse_wifi_status(text: &str) -> (String, Vec<(String, String)>) {
    let mut lines = text.lines();
    let state = lines
        .next()
        .and_then(|l| l.split_once(':'))
        .map(|(_, s)| s.trim().to_string())
        .unwrap_or_default();
    let fields = lines
        .filter_map(|l| {
            let l = l.trim();
            let (k, v) = l.split_once(char::is_whitespace)?;
            Some((k.to_string(), v.trim().to_string()))
        })
        .collect();
    (state, fields)
}

/// `wifi scan`: a header, then "BSSID  CHAN  SIGNAL  SECURITY  SSID" columns.
pub fn parse_wifi_scan(text: &str) -> Vec<WifiNetwork> {
    let mut out: Vec<WifiNetwork> = Vec::new();
    for l in text.lines().skip(1) {
        let mut w = l.split_whitespace();
        let (Some(bssid), Some(_chan), Some(signal), Some(security)) =
            (w.next(), w.next(), w.next(), w.next())
        else {
            continue;
        };
        let ssid = w.collect::<Vec<_>>().join(" ");
        if ssid.is_empty() || ssid == "(hidden)" {
            continue;
        }
        let signal = signal_percent(signal);
        match out.iter_mut().find(|n| n.ssid == ssid) {
            Some(n) if n.signal >= signal => {}
            Some(n) => {
                n.signal = signal;
                n.bssid = bssid.into();
            }
            None => out.push(WifiNetwork {
                ssid,
                signal,
                security: if security == "open" {
                    String::new()
                } else {
                    security.to_string()
                },
                in_use: false,
                bssid: bssid.into(),
            }),
        }
    }
    out.sort_by_key(|n| std::cmp::Reverse(n.signal));
    out
}

/// Signal as "-55" dBm (or "-55dBm", or a percentage) → 0..100.
fn signal_percent(s: &str) -> u8 {
    let t = s.trim_end_matches("dBm").trim_end_matches('%');
    match t.parse::<i32>() {
        Ok(v) if v < 0 => ((v + 100) * 2).clamp(0, 100) as u8,
        Ok(v) => v.clamp(0, 100) as u8,
        Err(_) => 0,
    }
}

/// Saved networks (ssid, psk) in wifi.conf.
pub fn parse_wifi_conf(text: &str) -> Vec<(String, String)> {
    let unquote = |v: &str| {
        let v = v.trim();
        v.strip_prefix('"')
            .and_then(|x| x.strip_suffix('"'))
            .unwrap_or(v)
            .to_string()
    };
    let mut out: Vec<(String, String)> = Vec::new();
    for line in text.lines().map(str::trim) {
        if let Some(v) = line.strip_prefix("ssid=") {
            out.push((unquote(v), String::new()));
        } else if let Some(v) = line.strip_prefix("psk=") {
            if let Some(last) = out.last_mut() {
                last.1 = unquote(v);
            }
        }
    }
    out
}

fn saved() -> Vec<(String, String)> {
    let text = std::fs::read_to_string("/storage/etc/wifi.conf")
        .or_else(|_| std::fs::read_to_string("/etc/wifi.conf"))
        .unwrap_or_default();
    parse_wifi_conf(&text)
}

/// The last scan, reused for a while: a scan takes seconds and the panel polls.
static SCAN: Mutex<Option<(Instant, Vec<WifiNetwork>)>> = Mutex::new(None);

pub fn query(r: &dyn CommandRunner, rescan: bool) -> NetworkState {
    let mut st = NetworkState::default();
    let ifaces = r
        .run("ip", &["-br", "addr"])
        .ok()
        .filter(|o| o.ok())
        .map(|o| parse_ip_brief(&o.stdout))
        .unwrap_or_default();
    st.available = !ifaces.is_empty();
    let online = ifaces
        .iter()
        .any(|i| i.name != "lo" && i.addrs.iter().any(|a| !a.contains(':')));
    st.connectivity = if online { "full" } else { "none" }.into();
    let wlan = ifaces.iter().find(|i| i.name.starts_with("wlan"));
    st.wifi_hardware = wlan.is_some();
    st.wifi_enabled = wlan.is_some();
    if wlan.is_some() {
        if let Ok(out) = r.run("wifi", &["status"]) {
            let (state, fields) = parse_wifi_status(&out.stdout);
            if state == "connected" {
                st.active_ssid = fields
                    .iter()
                    .find(|(k, _)| k == "ssid")
                    .map(|(_, v)| v.trim_matches('"').to_string());
            }
        }
        let mut cache = SCAN.lock().unwrap();
        let stale = cache
            .as_ref()
            .is_none_or(|(t, _)| t.elapsed() > Duration::from_secs(60));
        if rescan || stale {
            if let Ok(out) = r.run("wifi", &["scan"]) {
                if out.ok() {
                    *cache = Some((Instant::now(), parse_wifi_scan(&out.stdout)));
                }
            }
        }
        st.networks = cache.as_ref().map(|(_, n)| n.clone()).unwrap_or_default();
        for n in &mut st.networks {
            n.in_use = st.active_ssid.as_deref() == Some(n.ssid.as_str());
        }
        let device = wlan.map(|w| w.name.clone()).unwrap_or_default();
        for (ssid, _) in saved() {
            st.connections.push(Connection {
                active: st.active_ssid.as_deref() == Some(ssid.as_str()),
                name: ssid,
                kind: "802-11-wireless".into(),
                device: device.clone(),
                wg_public_key: String::new(),
            });
        }
    }
    for i in ifaces
        .iter()
        .filter(|i| i.name != "lo" && !i.name.starts_with("wlan"))
    {
        let active = i.carrier && !i.addrs.is_empty();
        if active {
            st.ethernet_connected = true;
        }
        st.connections.push(Connection {
            name: i.name.clone(),
            kind: "802-3-ethernet".into(),
            device: i.name.clone(),
            active,
            wg_public_key: String::new(),
        });
    }
    st
}

pub fn wifi_connect(r: &dyn CommandRunner, ssid: &str, password: Option<&str>) -> Result<()> {
    let mut args = vec!["connect", ssid];
    if let Some(p) = password.filter(|p| !p.is_empty()) {
        args.push(p);
    }
    args.push("--save");
    r.run_ok("wifi", &args).map(|_| ())
}

fn is_saved_wifi(name: &str) -> Option<String> {
    saved()
        .into_iter()
        .find(|(s, _)| s == name)
        .map(|(_, psk)| psk)
}

pub fn connection_up(r: &dyn CommandRunner, name: &str) -> Result<()> {
    if let Some(psk) = is_saved_wifi(name) {
        return wifi_connect(r, name, Some(&psk));
    }
    r.run_ok("ip", &["link", "set", name, "up"])?;
    r.run_ok("dhcp", &[name]).map(|_| ())
}

pub fn connection_down(r: &dyn CommandRunner, name: &str) -> Result<()> {
    if is_saved_wifi(name).is_some() {
        return r.run_ok("wifi", &["disconnect"]).map(|_| ());
    }
    r.run_ok("ip", &["link", "set", name, "down"]).map(|_| ())
}

pub fn connection_delete(r: &dyn CommandRunner, name: &str) -> Result<()> {
    if is_saved_wifi(name).is_none() {
        return Err(anyhow!("{name} is not a saved wireless network"));
    }
    r.run_ok("wifi", &["forget", name]).map(|_| ())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::runner::FakeRunner;

    #[test]
    fn parses_rustos_tools() {
        let ip = "lo       UP    127.0.0.1/8 ::1/128\neth0     UP    10.0.2.15/24 fec0::5054:ff:fe12:3456/64\nwlan0    NO-CARRIER \n";
        let ifs = parse_ip_brief(ip);
        assert_eq!(ifs.len(), 3);
        assert!(ifs[1].carrier && ifs[1].addrs[0] == "10.0.2.15/24");
        assert!(ifs[2].up && !ifs[2].carrier && ifs[2].addrs.is_empty());

        let (state, f) =
            parse_wifi_status("wlan0: connected\n  ssid     \"Home Net\"\n  signal   -48 dBm\n");
        assert_eq!(state, "connected");
        assert_eq!(f[0], ("ssid".into(), "\"Home Net\"".into()));

        let scan = "BSSID              CHAN  SIGNAL  SECURITY    SSID\n02:00:00:00:01:00     1     -70  wpa2        Home Net\n02:00:00:00:02:00     6     -80  open        Cafe\n02:00:00:00:03:00    11     -55  wpa2        Home Net\n02:00:00:00:04:00    11     -60  wpa3        (hidden)\n";
        let n = parse_wifi_scan(scan);
        assert_eq!(n.len(), 2);
        assert_eq!(n[0].ssid, "Home Net");
        assert_eq!(n[0].signal, 90);
        assert_eq!(n[0].bssid, "02:00:00:00:03:00");
        assert_eq!(n[1].security, "");

        let conf = "# saved\n\nssid=\"Home Net\"\npsk=\"secret pass\"\n\nssid=\"Cafe\"\n";
        assert_eq!(
            parse_wifi_conf(conf),
            vec![
                ("Home Net".to_string(), "secret pass".to_string()),
                ("Cafe".to_string(), String::new())
            ]
        );
    }

    #[test]
    fn query_without_wifi() {
        let r =
            FakeRunner::default().with("ip -br addr", "lo UP 127.0.0.1/8\neth0 UP 10.0.2.15/24\n");
        let st = query(&r, false);
        assert!(st.available && st.ethernet_connected && !st.wifi_hardware);
        assert_eq!(st.connectivity, "full");
        assert_eq!(st.connections.len(), 1);
        assert!(!r.calls().iter().any(|c| c.starts_with("wifi")));
    }

    #[test]
    fn connect_saves() {
        let r = FakeRunner::default().with("wifi", "wifi: connected\n");
        wifi_connect(&r, "Home Net", Some("pw123456")).unwrap();
        assert_eq!(r.calls(), ["wifi connect Home Net pw123456 --save"]);
    }
}
