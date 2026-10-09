//! Power on RustOS: batteries and AC adapters in `/sys/class/power_supply`, `poweroff` and
//! `reboot`. There is no suspend (no S3) and no power-profile daemon.

use std::path::Path;

use anyhow::{anyhow, Result};

use crate::{power::PowerState, runner::CommandRunner, LogindAction};

fn read(dir: &Path, name: &str) -> Option<String> {
    std::fs::read_to_string(dir.join(name))
        .ok()
        .map(|s| s.trim().to_string())
}

fn num(dir: &Path, name: &str) -> Option<f64> {
    read(dir, name).and_then(|s| s.parse().ok())
}

/// Read the supplies under `root` (normally `/sys/class/power_supply`).
pub fn query_at(root: &Path) -> PowerState {
    let mut st = PowerState {
        can_reboot: true,
        can_poweroff: true,
        ..Default::default()
    };
    let mut entries: Vec<_> = std::fs::read_dir(root)
        .map(|d| d.flatten().map(|e| e.path()).collect())
        .unwrap_or_default();
    entries.sort();
    let mut ac_seen = false;
    let (mut now_total, mut full_total, mut rate_total) = (0.0, 0.0, 0.0);
    let mut charging = false;
    let mut discharging = false;
    let mut percents = Vec::new();
    for dir in entries {
        match read(&dir, "type").as_deref() {
            Some("Mains") | Some("USB") => {
                ac_seen = true;
                if num(&dir, "online").unwrap_or(0.0) > 0.0 {
                    st.on_ac = true;
                }
            }
            Some("Battery") => {
                if read(&dir, "present").as_deref() == Some("0") {
                    continue;
                }
                st.battery_present = true;
                if let Some(c) = num(&dir, "capacity") {
                    percents.push(c);
                }
                // energy_* (µWh) or charge_* (µAh): both work for the ratio and the time.
                let now = num(&dir, "energy_now").or_else(|| num(&dir, "charge_now"));
                let full = num(&dir, "energy_full").or_else(|| num(&dir, "charge_full"));
                let rate = num(&dir, "power_now").or_else(|| num(&dir, "current_now"));
                if let (Some(n), Some(f)) = (now, full) {
                    now_total += n;
                    full_total += f;
                }
                rate_total += rate.unwrap_or(0.0).abs();
                match read(&dir, "status").as_deref() {
                    Some("Charging") => charging = true,
                    Some("Discharging") => discharging = true,
                    _ => {}
                }
            }
            _ => {}
        }
    }
    if st.battery_present {
        st.battery_percent = if full_total > 0.0 {
            (now_total * 100.0 / full_total).clamp(0.0, 100.0)
        } else if !percents.is_empty() {
            percents.iter().sum::<f64>() / percents.len() as f64
        } else {
            0.0
        };
        st.battery_state = if charging {
            "charging"
        } else if discharging {
            "discharging"
        } else if st.battery_percent >= 99.0 {
            "full"
        } else {
            "unknown"
        }
        .into();
        if rate_total > 0.0 {
            if discharging {
                st.time_to_empty_secs = (now_total / rate_total * 3600.0) as i64;
            } else if charging {
                st.time_to_full_secs =
                    ((full_total - now_total).max(0.0) / rate_total * 3600.0) as i64;
            }
        }
        if !ac_seen {
            st.on_ac = !discharging;
        }
    } else {
        st.on_ac = true;
    }
    st
}

pub fn query(_r: &dyn CommandRunner) -> PowerState {
    query_at(Path::new("/sys/class/power_supply"))
}

pub fn logind(r: &dyn CommandRunner, action: LogindAction) -> Result<()> {
    match action {
        LogindAction::Reboot => r.run_ok("reboot", &[]).map(|_| ()),
        LogindAction::PowerOff => r.run_ok("poweroff", &[]).map(|_| ()),
        LogindAction::Suspend | LogindAction::Hibernate => {
            Err(anyhow!("RustOS cannot suspend or hibernate yet"))
        }
        LogindAction::Lock => Err(anyhow!("RustOS has no screen locker")),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn supply(root: &Path, name: &str, files: &[(&str, &str)]) {
        let d = root.join(name);
        std::fs::create_dir_all(&d).unwrap();
        for (f, v) in files {
            std::fs::write(d.join(f), format!("{v}\n")).unwrap();
        }
    }

    #[test]
    fn battery_and_ac() {
        let t = tempfile::tempdir().unwrap();
        supply(t.path(), "AC", &[("type", "Mains"), ("online", "0")]);
        supply(
            t.path(),
            "BAT0",
            &[
                ("type", "Battery"),
                ("present", "1"),
                ("status", "Discharging"),
                ("capacity", "42"),
                ("energy_now", "21000000"),
                ("energy_full", "50000000"),
                ("power_now", "10500000"),
            ],
        );
        let st = query_at(t.path());
        assert!(st.battery_present && !st.on_ac);
        assert!((st.battery_percent - 42.0).abs() < 0.01);
        assert_eq!(st.battery_state, "discharging");
        assert_eq!(st.time_to_empty_secs, 7200);
        assert!(st.can_poweroff && !st.can_suspend);
    }

    #[test]
    fn desktop_without_battery() {
        let t = tempfile::tempdir().unwrap();
        let st = query_at(t.path());
        assert!(!st.battery_present && st.on_ac);
        assert!(st.profiles.is_empty());
    }
}
