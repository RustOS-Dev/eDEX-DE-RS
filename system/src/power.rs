//! Power: battery (UPower), profiles (power-profiles-daemon, when installed) and login1
//! actions. The lid policy is edex-comp's (`power.lid_close` in config.toml).

use anyhow::{Context, Result};
use zbus::blocking::Connection;

use crate::runner::CommandRunner;

#[derive(Clone, Debug, Default, PartialEq)]
pub struct PowerState {
    pub battery_present: bool,
    pub battery_percent: f64,
    /// charging | discharging | full | unknown
    pub battery_state: String,
    pub time_to_empty_secs: i64,
    pub time_to_full_secs: i64,
    pub on_ac: bool,
    pub profile: String,
    pub profiles: Vec<String>,
    pub can_suspend: bool,
    pub can_hibernate: bool,
    pub can_reboot: bool,
    pub can_poweroff: bool,
}

fn upower(conn: &Connection, st: &mut PowerState) -> Result<()> {
    let proxy = zbus::blocking::Proxy::new(
        conn,
        "org.freedesktop.UPower",
        "/org/freedesktop/UPower/devices/DisplayDevice",
        "org.freedesktop.UPower.Device",
    )?;
    let is_present: bool = proxy.get_property("IsPresent")?;
    st.battery_present = is_present;
    if is_present {
        st.battery_percent = proxy.get_property("Percentage")?;
        let state: u32 = proxy.get_property("State")?;
        st.battery_state = match state {
            1 => "charging",
            2 => "discharging",
            3 => "empty",
            4 => "full",
            5 => "pending charge",
            6 => "pending discharge",
            _ => "unknown",
        }
        .into();
        st.time_to_empty_secs = proxy.get_property("TimeToEmpty")?;
        st.time_to_full_secs = proxy.get_property("TimeToFull")?;
    }
    let up = zbus::blocking::Proxy::new(
        conn,
        "org.freedesktop.UPower",
        "/org/freedesktop/UPower",
        "org.freedesktop.UPower",
    )?;
    let on_battery: bool = up.get_property("OnBattery")?;
    st.on_ac = !on_battery;
    Ok(())
}

fn logind_can(conn: &Connection, method: &str) -> bool {
    zbus::blocking::Proxy::new(
        conn,
        "org.freedesktop.login1",
        "/org/freedesktop/login1",
        "org.freedesktop.login1.Manager",
    )
    .and_then(|p| p.call::<_, _, String>(method, &()))
    .map(|s| s == "yes" || s == "challenge")
    .unwrap_or(false)
}

pub fn query(r: &dyn CommandRunner) -> PowerState {
    let mut st = PowerState {
        profile: "balanced".into(),
        ..Default::default()
    };
    if let Ok(conn) = Connection::system() {
        if let Err(e) = upower(&conn, &mut st) {
            tracing::debug!("upower unavailable: {e}");
        }
        st.can_suspend = logind_can(&conn, "CanSuspend");
        st.can_hibernate = logind_can(&conn, "CanHibernate");
        st.can_reboot = logind_can(&conn, "CanReboot");
        st.can_poweroff = logind_can(&conn, "CanPowerOff");
    }
    if let Ok(out) = r.run("powerprofilesctl", &["get"]) {
        if out.ok() {
            st.profile = out.stdout.trim().to_string();
        }
    }
    if let Ok(out) = r.run("powerprofilesctl", &["list"]) {
        if out.ok() {
            st.profiles = out
                .stdout
                .lines()
                .filter_map(|l| {
                    let l = l.trim().trim_start_matches('*').trim();
                    l.strip_suffix(':').map(|s| s.to_string())
                })
                .collect();
        }
    }
    st
}

pub fn set_profile(r: &dyn CommandRunner, profile: &str) -> Result<()> {
    r.run_ok("powerprofilesctl", &["set", profile]).map(|_| ())
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum LogindAction {
    Suspend,
    Hibernate,
    Reboot,
    PowerOff,
    Lock,
}

pub fn logind(action: LogindAction) -> Result<()> {
    let conn = Connection::system().context("system bus")?;
    let manager = zbus::blocking::Proxy::new(
        &conn,
        "org.freedesktop.login1",
        "/org/freedesktop/login1",
        "org.freedesktop.login1.Manager",
    )?;
    match action {
        LogindAction::Suspend => manager.call::<_, _, ()>("Suspend", &(true,))?,
        LogindAction::Hibernate => manager.call::<_, _, ()>("Hibernate", &(true,))?,
        LogindAction::Reboot => manager.call::<_, _, ()>("Reboot", &(true,))?,
        LogindAction::PowerOff => manager.call::<_, _, ()>("PowerOff", &(true,))?,
        LogindAction::Lock => {
            let session_id = std::env::var("XDG_SESSION_ID").unwrap_or_else(|_| "auto".into());
            manager
                .call::<_, _, ()>("LockSession", &(session_id.as_str(),))
                .or_else(|_| manager.call::<_, _, ()>("LockSessions", &()))?
        }
    }
    Ok(())
}
