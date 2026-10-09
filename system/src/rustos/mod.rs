//! RustOS backends behind the same requests as the Linux ones: ALSA controls (or OSS through
//! `mixer`), `wifi`/`ip` (wpa_supplicant), `bt`, /sys power supplies and backlights,
//! `poweroff`/`reboot`, /etc/passwd and the commands /etc/rc starts.

pub mod audio;
pub mod bluetooth;
pub mod brightness;
pub mod network;
pub mod power;
pub mod services;
pub mod users;
