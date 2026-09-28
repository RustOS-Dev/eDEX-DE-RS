//! Tor, Tailscale, VPN and DNS control (eDEX-OS helpers + CLIs).

use std::{
    io::{BufRead, BufReader, Write},
    net::TcpStream,
    time::Duration,
};

use anyhow::{anyhow, Context, Result};
use serde::Deserialize;

use crate::runner::CommandRunner;

#[derive(Clone, Debug, Default, PartialEq)]
pub struct TorState {
    pub installed: bool,
    pub mode: String,
    pub running: bool,
    pub bootstrap_pct: u8,
    pub circuit_established: bool,
    pub bridges: String,
    pub version: String,
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct TailscalePeer {
    pub name: String,
    pub ip: String,
    pub os: String,
    pub online: bool,
    pub exit_node_option: bool,
    pub exit_node_active: bool,
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct TailscaleState {
    pub installed: bool,
    pub backend_state: String,
    pub self_name: String,
    pub self_ip: String,
    pub tailnet: String,
    pub peers: Vec<TailscalePeer>,
    pub exit_node: Option<String>,
    pub login_url: Option<String>,
    pub allow_lan: bool,
    pub advertise_exit: bool,
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct DnsState {
    pub resolver: String,
    pub dnscrypt_active: bool,
    pub test_result: String,
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct PrivacyState {
    pub tor: TorState,
    pub tailscale: TailscaleState,
    pub dns: DnsState,
    pub firewall_active: bool,
}

// ─── Tor ────────────────────────────────────────────────────────────────────

fn control_port(cmd: &str) -> Result<String> {
    let cookie = std::fs::read("/run/tor/control.authcookie")
        .or_else(|_| std::fs::read("/var/lib/tor/control_auth_cookie"))
        .context("tor control cookie unreadable")?;
    let hex: String = cookie.iter().map(|b| format!("{b:02x}")).collect();
    let mut stream = TcpStream::connect_timeout(
        &"127.0.0.1:9051".parse().unwrap(),
        Duration::from_millis(500),
    )
    .context("tor control port")?;
    stream.set_read_timeout(Some(Duration::from_secs(2)))?;
    stream.write_all(format!("AUTHENTICATE {hex}\r\n{cmd}\r\nQUIT\r\n").as_bytes())?;
    let mut reader = BufReader::new(stream);
    let mut out = String::new();
    let mut line = String::new();
    while reader.read_line(&mut line)? > 0 {
        out.push_str(&line);
        if line.starts_with("250 closing")
            || line.starts_with("5") && line.contains("Authentication")
        {
            break;
        }
        line.clear();
    }
    if out.contains("515 ") || out.contains("514 ") {
        return Err(anyhow!("tor control authentication failed"));
    }
    Ok(out)
}

pub fn tor_query(r: &dyn CommandRunner) -> TorState {
    let mut st = TorState {
        installed: std::path::Path::new("/usr/bin/tor").exists(),
        ..Default::default()
    };
    st.mode = std::fs::read_to_string("/run/edex-tor-mode")
        .map(|s| s.trim().to_string())
        .unwrap_or_else(|_| "off".into());
    st.running = sysmon::privacy::tcp_listening(9050);
    st.bridges = std::fs::read_to_string("/etc/tor/torrc.d/40-bridges.conf")
        .map(|t| {
            if t.contains("snowflake") {
                "snowflake".into()
            } else if t.contains("obfs4") {
                "obfs4".into()
            } else {
                "none".into()
            }
        })
        .unwrap_or_else(|_| "none".into());
    if st.running {
        if let Ok(out) =
            control_port("GETINFO status/bootstrap-phase status/circuit-established version")
        {
            for l in out.lines() {
                if let Some(rest) = l.split("PROGRESS=").nth(1) {
                    st.bootstrap_pct = rest
                        .split_whitespace()
                        .next()
                        .and_then(|p| p.parse().ok())
                        .unwrap_or(0);
                }
                if l.contains("status/circuit-established=") {
                    st.circuit_established = l.trim_end().ends_with('1');
                }
                if let Some(v) = l.strip_prefix("250-version=") {
                    st.version = v.trim().to_string();
                }
            }
        }
    }
    if st.version.is_empty() {
        if let Ok(out) = r.run("tor", &["--version"]) {
            st.version = out
                .stdout
                .lines()
                .next()
                .unwrap_or("")
                .replace("Tor version ", "")
                .trim_end_matches('.')
                .to_string();
        }
    }
    st
}

pub fn tor_set_mode(r: &dyn CommandRunner, mode: &str) -> Result<()> {
    if !["off", "socks5", "transparent"].contains(&mode) {
        return Err(anyhow!("invalid tor mode {mode}"));
    }
    r.run_ok("pkexec", &["/usr/bin/edex-tor-mode", mode])
        .map(|_| ())
}

pub fn tor_newnym() -> Result<()> {
    control_port("SIGNAL NEWNYM").map(|_| ())
}

/// `kind`: obfs4 (lines via stdin) | snowflake | clear.
pub fn tor_bridges(r: &dyn CommandRunner, kind: &str, lines: &str) -> Result<()> {
    match kind {
        "obfs4" => r
            .run_with_stdin(
                "pkexec",
                &["/usr/bin/edex-tor-bridges", "obfs4", "--stdin"],
                lines,
            )
            .and_then(|o| {
                if o.ok() {
                    Ok(())
                } else {
                    Err(anyhow!("{}", o.stderr.trim()))
                }
            }),
        "snowflake" | "clear" => r
            .run_ok("pkexec", &["/usr/bin/edex-tor-bridges", kind])
            .map(|_| ()),
        other => Err(anyhow!("unknown bridge kind {other}")),
    }
}

// ─── Tailscale ──────────────────────────────────────────────────────────────

#[derive(Deserialize)]
struct TsStatus {
    #[serde(rename = "BackendState", default)]
    backend_state: String,
    #[serde(rename = "Self", default)]
    self_node: Option<TsNode>,
    #[serde(rename = "Peer", default)]
    peers: std::collections::HashMap<String, TsNode>,
    #[serde(rename = "ExitNodeStatus", default)]
    exit_node_status: Option<TsExitStatus>,
    #[serde(rename = "MagicDNSSuffix", default)]
    magic_dns: String,
    #[serde(rename = "AuthURL", default)]
    auth_url: String,
}

#[derive(Deserialize, Clone)]
struct TsNode {
    #[serde(rename = "HostName", default)]
    host_name: String,
    #[serde(rename = "DNSName", default)]
    dns_name: String,
    #[serde(rename = "OS", default)]
    os: String,
    #[serde(rename = "TailscaleIPs", default)]
    ips: Vec<String>,
    #[serde(rename = "Online", default)]
    online: bool,
    #[serde(rename = "ExitNodeOption", default)]
    exit_node_option: bool,
    #[serde(rename = "ExitNode", default)]
    exit_node: bool,
}

#[derive(Deserialize)]
struct TsExitStatus {
    #[serde(rename = "ID", default)]
    id: String,
}

pub fn tailscale_query(r: &dyn CommandRunner) -> TailscaleState {
    let mut st = TailscaleState {
        installed: std::path::Path::new("/usr/bin/tailscale").exists(),
        ..Default::default()
    };
    let Ok(out) = r.run("tailscale", &["status", "--json"]) else {
        return st;
    };
    if !out.ok() {
        st.backend_state = "Stopped".into();
        return st;
    }
    let Ok(parsed) = serde_json::from_str::<TsStatus>(&out.stdout) else {
        return st;
    };
    st.backend_state = parsed.backend_state;
    st.tailnet = parsed.magic_dns;
    if let Some(me) = parsed.self_node {
        st.self_name = me.host_name;
        st.self_ip = me.ips.first().cloned().unwrap_or_default();
    }
    if !parsed.auth_url.is_empty() {
        st.login_url = Some(parsed.auth_url);
    }
    let _ = parsed.exit_node_status.map(|e| e.id);
    for (_id, p) in parsed.peers {
        let name = if p.host_name.is_empty() {
            p.dns_name.trim_end_matches('.').to_string()
        } else {
            p.host_name
        };
        if p.exit_node {
            st.exit_node = Some(name.clone());
        }
        st.peers.push(TailscalePeer {
            name,
            ip: p.ips.first().cloned().unwrap_or_default(),
            os: p.os,
            online: p.online,
            exit_node_option: p.exit_node_option,
            exit_node_active: p.exit_node,
        });
    }
    st.peers
        .sort_by(|a, b| b.online.cmp(&a.online).then(a.name.cmp(&b.name)));
    if let Ok(prefs) = r.run("tailscale", &["debug", "prefs"]) {
        if prefs.ok() {
            st.allow_lan = prefs.stdout.contains("\"ExitNodeAllowLANAccess\": true");
            st.advertise_exit = prefs.stdout.contains("0.0.0.0/0");
        }
    }
    st
}

pub fn tailscale_up(r: &dyn CommandRunner) -> Result<()> {
    r.run_ok("tailscale", &["up", "--timeout", "10s"])
        .map(|_| ())
}

pub fn tailscale_down(r: &dyn CommandRunner) -> Result<()> {
    r.run_ok("tailscale", &["down"]).map(|_| ())
}

/// Start an interactive login; returns the URL to open.
pub fn tailscale_login(r: &dyn CommandRunner) -> Result<String> {
    let out = r.run("tailscale", &["login", "--timeout", "3s"])?;
    let text = format!("{}\n{}", out.stdout, out.stderr);
    text.split_whitespace()
        .find(|w| w.starts_with("https://login.tailscale.com/"))
        .map(|s| s.to_string())
        .ok_or_else(|| anyhow!("no login URL returned: {}", text.trim()))
}

pub fn tailscale_exit_node(r: &dyn CommandRunner, node: Option<&str>) -> Result<()> {
    r.run_ok(
        "tailscale",
        &["set", &format!("--exit-node={}", node.unwrap_or(""))],
    )
    .map(|_| ())
}

pub fn tailscale_allow_lan(r: &dyn CommandRunner, allow: bool) -> Result<()> {
    r.run_ok(
        "tailscale",
        &["set", &format!("--exit-node-allow-lan-access={allow}")],
    )
    .map(|_| ())
}

pub fn tailscale_advertise_exit(r: &dyn CommandRunner, on: bool) -> Result<()> {
    r.run_ok(
        "tailscale",
        &["set", &format!("--advertise-exit-node={on}")],
    )
    .map(|_| ())
}

// ─── DNS / firewall ─────────────────────────────────────────────────────────

pub fn dns_query(r: &dyn CommandRunner) -> DnsState {
    let resolv = std::fs::read_to_string("/etc/resolv.conf").unwrap_or_default();
    let resolver = resolv
        .lines()
        .find_map(|l| {
            l.trim()
                .strip_prefix("nameserver")
                .map(|s| s.trim().to_string())
        })
        .unwrap_or_else(|| "none".into());
    let dnscrypt_active = sysmon::privacy::tcp_listening(53) && resolver.starts_with("127.");
    let test_result = match r.run("getent", &["hosts", "check.torproject.org"]) {
        Ok(o) if o.ok() => "resolves".into(),
        Ok(_) => "no resolution".into(),
        Err(_) => "unknown".into(),
    };
    DnsState {
        resolver,
        dnscrypt_active,
        test_result,
    }
}

pub fn firewall_active(r: &dyn CommandRunner) -> bool {
    crate::services::is_active(r, "nftables")
}

pub fn query(r: &dyn CommandRunner) -> PrivacyState {
    PrivacyState {
        tor: tor_query(r),
        tailscale: tailscale_query(r),
        dns: dns_query(r),
        firewall_active: firewall_active(r),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::runner::FakeRunner;

    #[test]
    fn parses_tailscale_status() {
        let json = r#"{"BackendState":"Running","AuthURL":"","MagicDNSSuffix":"tail.ts.net","Self":{"HostName":"laptop","DNSName":"laptop.tail.ts.net.","OS":"linux","TailscaleIPs":["100.64.0.1"],"Online":true},"Peer":{"a":{"HostName":"server","DNSName":"server.tail.ts.net.","OS":"linux","TailscaleIPs":["100.64.0.2"],"Online":true,"ExitNodeOption":true,"ExitNode":true},"b":{"HostName":"phone","OS":"iOS","TailscaleIPs":["100.64.0.3"],"Online":false}}}"#;
        let r = FakeRunner::default()
            .with("tailscale status --json", json)
            .with(
                "tailscale debug prefs",
                "{\"ExitNodeAllowLANAccess\": true}",
            );
        let s = tailscale_query(&r);
        assert_eq!(s.backend_state, "Running");
        assert_eq!(s.self_ip, "100.64.0.1");
        assert_eq!(s.peers.len(), 2);
        assert_eq!(s.peers[0].name, "server");
        assert_eq!(s.exit_node.as_deref(), Some("server"));
        assert!(s.allow_lan);
    }
}
