//! Bluetooth through RustOS's `/dev/bluetooth`: write one command line, read the kernel's
//! answer until end of file (the protocol the `bt` tool speaks). Lines starting with `? ` ask a
//! question that is answered by writing to the device.

use std::{
    fs::OpenOptions,
    io::{Read, Write},
};

use anyhow::{anyhow, Context, Result};

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

pub const DEVICE: &str = "/dev/bluetooth";

/// One command on the control device.
pub trait BtControl {
    /// Run `line`; `answer` replies to `? ` questions. Returns the output lines.
    fn run(&self, line: &str, answer: &dyn Fn(&str) -> Option<String>) -> Result<Vec<String>>;
}

pub struct DevBluetooth;

impl BtControl for DevBluetooth {
    fn run(&self, line: &str, answer: &dyn Fn(&str) -> Option<String>) -> Result<Vec<String>> {
        let mut f = OpenOptions::new()
            .read(true)
            .write(true)
            .open(DEVICE)
            .with_context(|| format!("{DEVICE} (no Bluetooth support)"))?;
        f.write_all(line.as_bytes())?;
        let mut out = Vec::new();
        let mut pending = Vec::new();
        let mut buf = [0u8; 512];
        loop {
            let n = match f.read(&mut buf) {
                Ok(0) => break,
                Ok(n) => n,
                // The kernel reports a failed command as an error on read.
                Err(e) => {
                    let detail = out.last().cloned().unwrap_or_default();
                    return Err(anyhow!("bt {line}: {e} {detail}"));
                }
            };
            pending.extend_from_slice(&buf[..n]);
            while let Some(i) = pending.iter().position(|&b| b == b'\n') {
                let raw: Vec<u8> = pending.drain(..=i).collect();
                let l = String::from_utf8_lossy(&raw[..raw.len() - 1]).into_owned();
                if let Some(q) = l.strip_prefix("? ") {
                    let reply = answer(q).unwrap_or_default();
                    f.write_all(reply.as_bytes())?;
                } else {
                    out.push(l);
                }
            }
        }
        if let Some(e) = out.iter().find(|l| l.starts_with("error: ")) {
            return Err(anyhow!("{}", e.trim_start_matches("error: ")));
        }
        Ok(out)
    }
}

fn no_answer(_: &str) -> Option<String> {
    None
}

/// Accept numeric comparison (the user sees the same code on the device); passkeys cannot be
/// typed from the panel.
fn pairing_answer(question: &str) -> Option<String> {
    question.contains("[yes/no]").then(|| "yes".to_string())
}

fn is_addr(s: &str) -> bool {
    s.len() == 17 && s.split(':').count() == 6
}

/// `ADDR  le    -60 dBm  keyboard  Name (random)` from `devices`.
fn parse_found(line: &str) -> Option<BtDevice> {
    let mut w = line.split_whitespace();
    let addr = w.next().filter(|a| is_addr(a))?;
    let _transport = w.next()?;
    let _rssi = w.next()?;
    if w.next()? != "dBm" {
        return None;
    }
    let kind = w.next().unwrap_or("").to_string();
    let name = w
        .collect::<Vec<_>>()
        .join(" ")
        .trim_end_matches("(random)")
        .trim()
        .to_string();
    Some(BtDevice {
        mac: addr.to_string(),
        name,
        icon: icon_for(&kind),
        ..Default::default()
    })
}

/// `ADDR  le    connected Name` from `list`.
fn parse_paired(line: &str) -> Option<BtDevice> {
    let mut w = line.split_whitespace();
    let addr = w.next().filter(|a| is_addr(a))?;
    let _transport = w.next()?;
    let state = w.next()?;
    Some(BtDevice {
        mac: addr.to_string(),
        name: w.collect::<Vec<_>>().join(" "),
        paired: true,
        trusted: true,
        connected: state == "connected",
        ..Default::default()
    })
}

fn icon_for(kind: &str) -> String {
    match kind {
        "keyboard" => "input-keyboard",
        "mouse" | "pointer" => "input-mouse",
        "gamepad" | "joystick" => "input-gaming",
        "headset" | "audio" => "audio-headset",
        "phone" => "phone",
        _ => "bluetooth",
    }
    .into()
}

pub fn query_with(bt: &dyn BtControl) -> BluetoothState {
    let mut st = BluetoothState::default();
    let Ok(status) = bt.run("status", &no_answer) else {
        return st;
    };
    let Some(first) = status.first().filter(|l| !l.starts_with("no Bluetooth")) else {
        return st;
    };
    st.available = true;
    // "hci0: AA:BB:CC:DD:EE:FF via usb, Bluetooth 5.2, manufacturer 0x0002, up, LE, BR/EDR"
    st.powered = first.split(", ").any(|p| p == "up");
    st.adapter_name = first
        .split_once(" via ")
        .map(|(a, _)| a.to_string())
        .unwrap_or_else(|| first.clone());
    let mut devices: Vec<BtDevice> = bt
        .run("list", &no_answer)
        .unwrap_or_default()
        .iter()
        .filter_map(|l| parse_paired(l))
        .collect();
    if st.powered {
        for d in bt
            .run("devices", &no_answer)
            .unwrap_or_default()
            .iter()
            .filter_map(|l| parse_found(l))
        {
            match devices.iter_mut().find(|p| p.mac == d.mac) {
                Some(p) => {
                    if p.name.is_empty() {
                        p.name = d.name;
                    }
                    p.icon = d.icon;
                }
                None => devices.push(d),
            }
        }
    }
    devices.sort_by(|a, b| {
        b.connected
            .cmp(&a.connected)
            .then(b.paired.cmp(&a.paired))
            .then(a.name.cmp(&b.name))
    });
    st.devices = devices;
    st
}

pub fn query() -> BluetoothState {
    query_with(&DevBluetooth)
}

pub fn power(on: bool) -> Result<()> {
    DevBluetooth
        .run(if on { "power on" } else { "power off" }, &no_answer)
        .map(|_| ())
}

/// Scan for a few seconds (the kernel blocks for the duration); stopping is implicit.
pub fn scan(on: bool) -> Result<()> {
    if !on {
        return Ok(());
    }
    DevBluetooth.run("scan 8", &no_answer).map(|_| ())
}

pub fn pair(mac: &str) -> Result<()> {
    DevBluetooth
        .run(&format!("pair {mac}"), &pairing_answer)
        .map(|_| ())
}

pub fn connect(mac: &str) -> Result<()> {
    DevBluetooth
        .run(&format!("connect {mac}"), &pairing_answer)
        .map(|_| ())
}

pub fn disconnect(mac: &str) -> Result<()> {
    DevBluetooth
        .run(&format!("disconnect {mac}"), &no_answer)
        .map(|_| ())
}

pub fn remove(mac: &str) -> Result<()> {
    DevBluetooth
        .run(&format!("remove {mac}"), &no_answer)
        .map(|_| ())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashMap;

    struct Fake(HashMap<&'static str, Vec<&'static str>>);

    impl BtControl for Fake {
        fn run(&self, line: &str, _a: &dyn Fn(&str) -> Option<String>) -> Result<Vec<String>> {
            self.0
                .get(line)
                .map(|v| v.iter().map(|s| s.to_string()).collect())
                .ok_or_else(|| anyhow!("unexpected {line}"))
        }
    }

    #[test]
    fn parses_the_kernel_output() {
        let fake = Fake(HashMap::from([
            (
                "status",
                vec![
                    "hci0: 11:22:33:44:55:66 via usb, Bluetooth 5.3, manufacturer 0x0002, up, LE, BR/EDR",
                    "  connected AA:BB:CC:DD:EE:FF (LE, encrypted)",
                ],
            ),
            (
                "list",
                vec![
                    "AA:BB:CC:DD:EE:FF  le    connected MX Keys",
                    "01:02:03:04:05:06  bredr paired    ",
                ],
            ),
            (
                "devices",
                vec![
                    "AA:BB:CC:DD:EE:FF  le     -50 dBm  keyboard  MX Keys",
                    "01:02:03:04:05:06  bredr  -70 dBm  mouse     Old Mouse",
                    "0A:0B:0C:0D:0E:0F  le     -80 dBm  unknown   Beacon (random)",
                ],
            ),
        ]));
        let s = query_with(&fake);
        assert!(s.available && s.powered);
        assert_eq!(s.adapter_name, "hci0: 11:22:33:44:55:66");
        assert_eq!(s.devices.len(), 3);
        assert_eq!(s.devices[0].name, "MX Keys");
        assert!(s.devices[0].connected && s.devices[0].paired);
        assert_eq!(s.devices[0].icon, "input-keyboard");
        let mouse = s
            .devices
            .iter()
            .find(|d| d.mac == "01:02:03:04:05:06")
            .unwrap();
        assert_eq!(mouse.name, "Old Mouse");
        assert!(mouse.paired && !mouse.connected);
        let beacon = s.devices.iter().find(|d| d.name == "Beacon").unwrap();
        assert!(!beacon.paired);

        let none = Fake(HashMap::from([("status", vec!["no Bluetooth controller"])]));
        assert!(!query_with(&none).available);
        assert_eq!(
            pairing_answer("Does the device show 123456? [yes/no]").as_deref(),
            Some("yes")
        );
        assert_eq!(pairing_answer("Passkey shown on the device:"), None);
    }
}
