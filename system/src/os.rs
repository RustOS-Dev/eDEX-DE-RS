//! Which operating system the shell runs on, and what its backends can do. Settings rows whose
//! backend is missing are hidden.

use std::sync::OnceLock;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Os {
    /// Linux with systemd/logind, NetworkManager, PipeWire, BlueZ (eDEX-OS, Arch, …).
    Linux,
    /// RustOS: its own tools (`wifi`, `ip`, `bt`, `mixer`), ALSA, /sys and /etc/rc.
    RustOs,
}

impl Os {
    /// `EDEX_SYSTEM=rustos|linux` overrides; otherwise `/proc/version` tells.
    pub fn detect() -> Os {
        static OS: OnceLock<Os> = OnceLock::new();
        *OS.get_or_init(|| {
            match std::env::var("EDEX_SYSTEM")
                .unwrap_or_default()
                .to_ascii_lowercase()
                .as_str()
            {
                "rustos" => return Os::RustOs,
                "linux" => return Os::Linux,
                _ => {}
            }
            let version = std::fs::read_to_string("/proc/version").unwrap_or_default();
            if version.starts_with("RustOS") {
                Os::RustOs
            } else {
                Os::Linux
            }
        })
    }

    pub fn name(self) -> &'static str {
        match self {
            Os::Linux => "Linux",
            Os::RustOs => "RustOS",
        }
    }
}

/// Features with a backend on this system.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Capabilities {
    pub os: Os,
    /// Choosing the default output/input device.
    pub audio_devices: bool,
    /// An external mixer application (pavucontrol).
    pub audio_mixer_app: bool,
    pub microphone: bool,
    pub wifi_radio: bool,
    pub airplane: bool,
    pub vpn: bool,
    pub power_profiles: bool,
    pub suspend: bool,
    pub hibernate: bool,
    pub lid_action: bool,
    /// hyprlock / logind session locking.
    pub lock: bool,
    /// Idle dimming, locking and screen blanking (hypridle).
    pub idle: bool,
    pub fingerprint: bool,
    pub tor: bool,
    pub tailscale: bool,
    pub firewall: bool,
    pub keyring: bool,
    pub user_edit: bool,
    pub user_services: bool,
    pub service_enable: bool,
}

impl Capabilities {
    pub fn for_os(os: Os) -> Self {
        match os {
            Os::Linux => Self {
                os,
                audio_devices: true,
                audio_mixer_app: true,
                microphone: true,
                wifi_radio: true,
                airplane: true,
                vpn: true,
                power_profiles: true,
                suspend: true,
                hibernate: true,
                lid_action: true,
                lock: true,
                idle: true,
                fingerprint: true,
                tor: true,
                tailscale: true,
                firewall: true,
                keyring: true,
                user_edit: true,
                user_services: true,
                service_enable: true,
            },
            // No suspend/hibernate (no S3), no Tor, Tailscale or fprintd, no NetworkManager
            // VPNs, no logind/hyprlock/hypridle, no systemd units.
            Os::RustOs => Self {
                os,
                audio_devices: false,
                audio_mixer_app: false,
                microphone: true,
                wifi_radio: false,
                airplane: false,
                vpn: false,
                power_profiles: false,
                suspend: false,
                hibernate: false,
                lid_action: false,
                lock: false,
                idle: false,
                fingerprint: false,
                tor: false,
                tailscale: false,
                firewall: false,
                keyring: false,
                user_edit: false,
                user_services: false,
                service_enable: false,
            },
        }
    }

    pub fn current() -> Self {
        Self::for_os(Os::detect())
    }
}

/// Shown where a hidden feature would be.
pub fn unavailable_note(os: Os) -> String {
    format!("not available on {}", os.name())
}
