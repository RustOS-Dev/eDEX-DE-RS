//! Privacy indicators without spawning processes: interfaces, sockets and open device files.

use std::{fs, path::Path, time::Instant};

#[derive(Clone, Debug, Default, PartialEq)]
pub struct PrivacyStatus {
    /// off | socks5 | transparent (from /run/edex-tor-mode when present).
    pub tor_mode: String,
    pub tor_active: bool,
    pub tailscale_connected: bool,
    pub wireguard_active: bool,
    pub vpn_active: bool,
    pub fprintd_active: bool,
    pub mic_active: bool,
    pub camera_active: bool,
}

/// Runs the cheap checks every call and the `/proc/*/fd` walk at most every `device_interval`.
pub struct PrivacyProbe {
    last_devices: Option<Instant>,
    mic: bool,
    camera: bool,
    device_interval_secs: u64,
}

impl Default for PrivacyProbe {
    fn default() -> Self {
        Self::new(5)
    }
}

impl PrivacyProbe {
    pub fn new(device_interval_secs: u64) -> Self {
        Self {
            last_devices: None,
            mic: false,
            camera: false,
            device_interval_secs,
        }
    }

    pub fn probe(&mut self, fprintd_running: bool) -> PrivacyStatus {
        let now = Instant::now();
        if self
            .last_devices
            .is_none_or(|t| now.duration_since(t).as_secs() >= self.device_interval_secs)
        {
            self.mic = device_in_use("/dev/snd/", "c");
            self.camera = device_in_use("/dev/video", "");
            self.last_devices = Some(now);
        }
        let tor_mode = fs::read_to_string("/run/edex-tor-mode")
            .map(|s| s.trim().to_string())
            .unwrap_or_default();
        let tor_listening = tcp_listening(9050);
        let tor_active = if tor_mode.is_empty() {
            tor_listening
        } else {
            tor_mode != "off" && tor_listening
        };
        // tailscaled keeps tailscale0 up while logged out; it only gets an address when connected.
        let tailscale = interface_up("tailscale0") && has_ipv4("tailscale0");
        let wireguard = wireguard_interfaces();
        let nm_vpn = nm_vpn_active();
        PrivacyStatus {
            tor_mode: if tor_mode.is_empty() {
                if tor_listening {
                    "socks5".into()
                } else {
                    "off".into()
                }
            } else {
                tor_mode
            },
            tor_active,
            tailscale_connected: tailscale,
            wireguard_active: wireguard,
            vpn_active: tailscale || wireguard || nm_vpn,
            fprintd_active: fprintd_running,
            mic_active: self.mic,
            camera_active: self.camera,
        }
    }
}

fn interface_up(name: &str) -> bool {
    fs::read_to_string(format!("/sys/class/net/{name}/operstate"))
        .map(|s| matches!(s.trim(), "up" | "unknown"))
        .unwrap_or(false)
        && fs::read_to_string(format!("/sys/class/net/{name}/carrier"))
            .map(|s| s.trim() == "1")
            .unwrap_or(true)
}

fn wireguard_interfaces() -> bool {
    let Ok(dir) = fs::read_dir("/sys/class/net") else {
        return false;
    };
    dir.flatten().any(|e| {
        let name = e.file_name().to_string_lossy().to_string();
        let is_wg = name.starts_with("wg")
            || fs::read_to_string(e.path().join("uevent"))
                .map(|u| u.contains("DEVTYPE=wireguard"))
                .unwrap_or(false);
        is_wg && interface_up(&name) && has_ipv4(&name)
    })
}

/// NetworkManager VPN (OpenVPN) plugins create `tun*` devices whose uevent names them.
fn nm_vpn_active() -> bool {
    let Ok(dir) = fs::read_dir("/sys/class/net") else {
        return false;
    };
    dir.flatten().any(|e| {
        let name = e.file_name().to_string_lossy().to_string();
        (name.starts_with("tun") || name.starts_with("ppp"))
            && interface_up(&name)
            && has_ipv4(&name)
    })
}

/// Whether interface `name` has an IPv4 address (tunnels exist before they are connected).
fn has_ipv4(name: &str) -> bool {
    let mut head: *mut libc::ifaddrs = std::ptr::null_mut();
    // SAFETY: getifaddrs fills `head` with a list we walk read-only and free once.
    if unsafe { libc::getifaddrs(&mut head) } != 0 {
        return false;
    }
    let mut found = false;
    let mut cur = head;
    while !cur.is_null() {
        // SAFETY: `cur` points into the list returned by getifaddrs.
        let ifa = unsafe { &*cur };
        if !ifa.ifa_addr.is_null() && !ifa.ifa_name.is_null() {
            // SAFETY: non-null pointers from getifaddrs; ifa_name is a C string.
            let family = unsafe { (*ifa.ifa_addr).sa_family } as i32;
            let ifname = unsafe { std::ffi::CStr::from_ptr(ifa.ifa_name) };
            if family == libc::AF_INET && ifname.to_bytes() == name.as_bytes() {
                found = true;
                break;
            }
        }
        cur = ifa.ifa_next;
    }
    // SAFETY: freeing the list getifaddrs allocated.
    unsafe { libc::freeifaddrs(head) };
    found
}

/// Whether something listens on 127.0.0.1:`port` (parses /proc/net/tcp).
pub fn tcp_listening(port: u16) -> bool {
    let Ok(tcp) = fs::read_to_string("/proc/net/tcp") else {
        return false;
    };
    let hex = format!(":{:04X}", port);
    tcp.lines().skip(1).any(|l| {
        let cols: Vec<&str> = l.split_whitespace().collect();
        cols.len() > 3 && cols[1].ends_with(&hex) && cols[3] == "0A"
    })
}

fn device_in_use(prefix: &str, suffix: &str) -> bool {
    let Ok(proc_dir) = fs::read_dir("/proc") else {
        return false;
    };
    let me = std::process::id().to_string();
    for entry in proc_dir.flatten() {
        let name = entry.file_name();
        let name = name.to_string_lossy();
        if !name.chars().all(|c| c.is_ascii_digit()) || name == me {
            continue;
        }
        let fd_dir = entry.path().join("fd");
        let Ok(fds) = fs::read_dir(&fd_dir) else {
            continue;
        };
        for fd in fds.flatten() {
            if let Ok(target) = fs::read_link(fd.path()) {
                let t = target.to_string_lossy();
                if t.starts_with(prefix)
                    && (suffix.is_empty() || t.ends_with(suffix))
                    && Path::new(&*t).exists()
                {
                    return true;
                }
            }
        }
    }
    false
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn probe_runs_without_privileges() {
        let mut p = PrivacyProbe::new(5);
        let s = p.probe(false);
        assert!(["off", "socks5", "transparent"].contains(&s.tor_mode.as_str()));
        assert!(!tcp_listening(1));
    }
}
