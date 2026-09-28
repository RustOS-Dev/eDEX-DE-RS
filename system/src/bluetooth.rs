//! Bluetooth via `bluetoothctl` (non-interactive subcommands).

use anyhow::Result;

use crate::runner::CommandRunner;

#[derive(Clone, Debug, Default, PartialEq)]
pub struct BtDevice {
    pub mac: String,
    pub name: String,
    pub paired: bool,
    pub connected: bool,
    pub trusted: bool,
    pub battery: Option<u8>,
    pub icon: String,
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct BluetoothState {
    pub available: bool,
    pub powered: bool,
    pub discovering: bool,
    pub adapter_name: String,
    pub devices: Vec<BtDevice>,
}

fn parse_info(text: &str, dev: &mut BtDevice) {
    for line in text.lines() {
        let l = line.trim();
        if let Some(v) = l.strip_prefix("Name:") {
            dev.name = v.trim().to_string();
        } else if let Some(v) = l.strip_prefix("Paired:") {
            dev.paired = v.trim() == "yes";
        } else if let Some(v) = l.strip_prefix("Connected:") {
            dev.connected = v.trim() == "yes";
        } else if let Some(v) = l.strip_prefix("Trusted:") {
            dev.trusted = v.trim() == "yes";
        } else if let Some(v) = l.strip_prefix("Icon:") {
            dev.icon = v.trim().to_string();
        } else if let Some(v) = l.strip_prefix("Battery Percentage:") {
            // "0x50 (80)"
            dev.battery = v
                .split('(')
                .nth(1)
                .and_then(|s| s.trim_end_matches(')').trim().parse().ok());
        }
    }
}

pub fn query(r: &dyn CommandRunner) -> BluetoothState {
    let mut st = BluetoothState::default();
    let Ok(show) = r.run("bluetoothctl", &["show"]) else {
        return st;
    };
    if !show.ok() || show.stdout.trim().is_empty() {
        return st;
    }
    st.available = true;
    for line in show.stdout.lines() {
        let l = line.trim();
        if let Some(v) = l.strip_prefix("Powered:") {
            st.powered = v.trim() == "yes";
        } else if let Some(v) = l.strip_prefix("Discovering:") {
            st.discovering = v.trim() == "yes";
        } else if let Some(v) = l.strip_prefix("Alias:") {
            st.adapter_name = v.trim().to_string();
        }
    }
    if let Ok(out) = r.run("bluetoothctl", &["devices"]) {
        for line in out.stdout.lines() {
            // "Device AA:BB:CC:DD:EE:FF Name"
            let mut parts = line.splitn(3, ' ');
            if parts.next() != Some("Device") {
                continue;
            }
            let Some(mac) = parts.next() else { continue };
            let mut dev = BtDevice {
                mac: mac.to_string(),
                name: parts.next().unwrap_or("").to_string(),
                ..Default::default()
            };
            if let Ok(info) = r.run("bluetoothctl", &["info", mac]) {
                parse_info(&info.stdout, &mut dev);
            }
            st.devices.push(dev);
        }
        st.devices.sort_by(|a, b| {
            b.connected
                .cmp(&a.connected)
                .then(b.paired.cmp(&a.paired))
                .then(a.name.cmp(&b.name))
        });
    }
    st
}

pub fn power(r: &dyn CommandRunner, on: bool) -> Result<()> {
    r.run_ok("bluetoothctl", &["power", if on { "on" } else { "off" }])
        .map(|_| ())
}

pub fn scan(r: &dyn CommandRunner, on: bool) -> Result<()> {
    // `scan on` blocks in interactive mode; use the timeout-bounded form.
    if on {
        r.run_ok("bluetoothctl", &["--timeout", "8", "scan", "on"])
            .map(|_| ())
    } else {
        r.run_ok("bluetoothctl", &["scan", "off"]).map(|_| ())
    }
}

pub fn pair(r: &dyn CommandRunner, mac: &str) -> Result<()> {
    r.run_ok("bluetoothctl", &["--timeout", "30", "pair", mac])?;
    r.run_ok("bluetoothctl", &["trust", mac]).map(|_| ())
}

pub fn connect(r: &dyn CommandRunner, mac: &str) -> Result<()> {
    r.run_ok("bluetoothctl", &["--timeout", "20", "connect", mac])
        .map(|_| ())
}

pub fn disconnect(r: &dyn CommandRunner, mac: &str) -> Result<()> {
    r.run_ok("bluetoothctl", &["disconnect", mac]).map(|_| ())
}

pub fn remove(r: &dyn CommandRunner, mac: &str) -> Result<()> {
    r.run_ok("bluetoothctl", &["remove", mac]).map(|_| ())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::runner::FakeRunner;

    #[test]
    fn parses_bluetoothctl() {
        let r = FakeRunner::default()
            .with("bluetoothctl show", "Controller 11:22 (public)\n\tAlias: laptop\n\tPowered: yes\n\tDiscovering: no\n")
            .with("bluetoothctl devices", "Device AA:BB:CC:DD:EE:FF WH-1000XM4\n")
            .with("bluetoothctl info AA:BB:CC:DD:EE:FF", "Device AA:BB:CC:DD:EE:FF (public)\n\tName: WH-1000XM4\n\tIcon: audio-headset\n\tPaired: yes\n\tTrusted: yes\n\tConnected: yes\n\tBattery Percentage: 0x50 (80)\n");
        let s = query(&r);
        assert!(s.available && s.powered);
        assert_eq!(s.adapter_name, "laptop");
        assert_eq!(s.devices[0].battery, Some(80));
        assert!(s.devices[0].connected);
    }
}
