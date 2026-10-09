//! Bluetooth on RustOS through its `bt` tool (the kernel's own Bluetooth stack).

use std::sync::Mutex;

use anyhow::Result;

use crate::{
    bluetooth::{BluetoothState, BtDevice},
    runner::CommandRunner,
};

/// `bt status`: "hci0: AA:BB:… via usb, Bluetooth 5.2, manufacturer 0x0002, up, LE, BR/EDR"
/// and "  connected ADDR (LE)" lines; or "no Bluetooth controller".
pub fn parse_status(text: &str) -> Option<(String, bool, Vec<String>)> {
    let mut lines = text.lines();
    let first = lines.next()?;
    let (name, rest) = first.split_once(':')?;
    if first.starts_with("no Bluetooth") {
        return None;
    }
    let up = rest.split(',').any(|p| p.trim() == "up");
    let connected = lines
        .filter_map(|l| l.trim().strip_prefix("connected "))
        .filter_map(|l| l.split_whitespace().next())
        .map(String::from)
        .collect();
    Some((name.trim().to_string(), up, connected))
}

/// `bt list`: "ADDR  le|bredr connected|paired NAME".
pub fn parse_list(text: &str) -> Vec<BtDevice> {
    text.lines()
        .filter_map(|l| {
            let mut w = l.split_whitespace();
            let mac = w.next()?;
            if mac.matches(':').count() != 5 {
                return None;
            }
            let _kind = w.next()?;
            let state = w.next()?;
            Some(BtDevice {
                mac: mac.into(),
                name: w.collect::<Vec<_>>().join(" "),
                paired: true,
                connected: state == "connected",
                trusted: true,
                battery: None,
                icon: String::new(),
            })
        })
        .collect()
}

/// `bt scan` / `bt devices`: "ADDR  le|bredr  -50 dBm  KIND  NAME [(random)]".
pub fn parse_devices(text: &str) -> Vec<BtDevice> {
    text.lines()
        .filter_map(|l| {
            let mut w = l.split_whitespace();
            let mac = w.next()?;
            if mac.matches(':').count() != 5 {
                return None;
            }
            let _kind = w.next()?;
            let _rssi = w.next()?;
            if w.next()? != "dBm" {
                return None;
            }
            let icon = w.next().unwrap_or("").to_string();
            let name: Vec<&str> = w.filter(|x| *x != "(random)").collect();
            Some(BtDevice {
                mac: mac.into(),
                name: name.join(" "),
                paired: false,
                connected: false,
                trusted: false,
                battery: None,
                icon,
            })
        })
        .collect()
}

/// Devices the last scan found.
static FOUND: Mutex<Vec<BtDevice>> = Mutex::new(Vec::new());

pub fn query(r: &dyn CommandRunner) -> BluetoothState {
    let mut st = BluetoothState::default();
    let Ok(out) = r.run("bt", &["status"]) else {
        return st;
    };
    let Some((name, up, connected)) = parse_status(&out.stdout) else {
        return st;
    };
    st.available = true;
    st.powered = up;
    st.adapter_name = name;
    if let Ok(out) = r.run("bt", &["list"]) {
        st.devices = parse_list(&out.stdout);
    }
    for d in &mut st.devices {
        if connected.iter().any(|c| c.eq_ignore_ascii_case(&d.mac)) {
            d.connected = true;
        }
    }
    for f in FOUND.lock().unwrap().iter() {
        if !st
            .devices
            .iter()
            .any(|d| d.mac.eq_ignore_ascii_case(&f.mac))
        {
            st.devices.push(f.clone());
        }
    }
    st
}

pub fn power(r: &dyn CommandRunner, on: bool) -> Result<()> {
    r.run_ok("bt", &["power", if on { "on" } else { "off" }])
        .map(|_| ())
}

/// Scan for 8 seconds (`bt scan` blocks); the devices found are listed until the next scan.
pub fn scan(r: &dyn CommandRunner, on: bool) -> Result<()> {
    if !on {
        return Ok(());
    }
    let out = r.run_ok("bt", &["scan", "8"])?;
    *FOUND.lock().unwrap() = parse_devices(&out);
    Ok(())
}

pub fn pair(r: &dyn CommandRunner, mac: &str) -> Result<()> {
    r.run_ok("bt", &["pair", mac]).map(|_| ())
}

pub fn connect(r: &dyn CommandRunner, mac: &str) -> Result<()> {
    r.run_ok("bt", &["connect", mac]).map(|_| ())
}

pub fn disconnect(r: &dyn CommandRunner, mac: &str) -> Result<()> {
    r.run_ok("bt", &["disconnect", mac]).map(|_| ())
}

pub fn remove(r: &dyn CommandRunner, mac: &str) -> Result<()> {
    r.run_ok("bt", &["remove", mac]).map(|_| ())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::runner::FakeRunner;

    #[test]
    fn parses_bt_output() {
        let status = "hci0: 00:1A:7D:DA:71:13 via usb, Bluetooth 5.0, manufacturer 0x000a, up, LE, BR/EDR\n  connected 11:22:33:44:55:66 (LE, encrypted)\n";
        let (name, up, conn) = parse_status(status).unwrap();
        assert_eq!(name, "hci0");
        assert!(up);
        assert_eq!(conn, ["11:22:33:44:55:66"]);
        assert!(parse_status("no Bluetooth controller\n").is_none());

        let list = "11:22:33:44:55:66  le    paired    MX Keys\nAA:BB:CC:DD:EE:FF  bredr connected Headset\n";
        let l = parse_list(list);
        assert_eq!(l.len(), 2);
        assert_eq!(l[0].name, "MX Keys");
        assert!(l[1].connected);
        assert!(parse_list("no paired devices\n").is_empty());

        let scan = "scanning for 8 s...\n11:22:33:44:55:66  le     -52 dBm  keyboard  MX Keys (random)\n22:33:44:55:66:77  bredr  -70 dBm  phone    \n";
        let d = parse_devices(scan);
        assert_eq!(d.len(), 2);
        assert_eq!(d[0].name, "MX Keys");
        assert_eq!(d[0].icon, "keyboard");
    }

    #[test]
    fn query_merges_paired_and_connected() {
        let r = FakeRunner::default()
            .with(
                "bt status",
                "hci0: 00:1A:7D:DA:71:13 via usb, Bluetooth 5.0, manufacturer 0x000a, down, LE\n",
            )
            .with("bt list", "11:22:33:44:55:66  le    paired    MX Keys\n");
        let st = query(&r);
        assert!(st.available && !st.powered);
        assert_eq!(st.devices.len(), 1);
    }
}
