//! Monitor identity from the EDID blob of a DRM connector.
//!
//! Only the base block is read: the manufacturer PNP id, the monitor name (descriptor 0xFC) and
//! the serial string (descriptor 0xFF), falling back to the numeric product code and serial.

use smithay::reexports::drm::control::{connector, property, Device};

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct EdidInfo {
    pub make: String,
    pub model: String,
    pub serial: String,
}

/// Read and parse the connector's `EDID` property.
pub fn for_connector(device: &impl Device, connector: connector::Handle) -> Option<EdidInfo> {
    let props = device.get_properties(connector).ok()?;
    let (ids, values) = props.as_props_and_values();
    for (id, value) in ids.iter().zip(values) {
        let info = device.get_property(*id).ok()?;
        if info.name().to_str() != Ok("EDID") {
            continue;
        }
        let property::Value::Blob(blob) = info.value_type().convert_value(*value) else {
            return None;
        };
        if blob == 0 {
            return None;
        }
        let data = device.get_property_blob(blob).ok()?;
        return parse(&data);
    }
    None
}

pub fn parse(edid: &[u8]) -> Option<EdidInfo> {
    const HEADER: [u8; 8] = [0x00, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0x00];
    if edid.len() < 128 || edid[..8] != HEADER {
        return None;
    }
    let id = u16::from_be_bytes([edid[8], edid[9]]);
    let letter = |shift: u16| (b'A' - 1 + ((id >> shift) & 0x1f) as u8) as char;
    let make = [letter(10), letter(5), letter(0)]
        .iter()
        .collect::<String>();
    let product = u16::from_le_bytes([edid[10], edid[11]]);
    let serial_number = u32::from_le_bytes([edid[12], edid[13], edid[14], edid[15]]);

    let mut model = None;
    let mut serial = None;
    for d in 0..4 {
        let block = &edid[54 + d * 18..54 + (d + 1) * 18];
        // Display descriptors start with a zero pixel clock.
        if block[0] != 0 || block[1] != 0 {
            continue;
        }
        let text = || {
            let raw = &block[5..18];
            let end = raw.iter().position(|&b| b == b'\n').unwrap_or(raw.len());
            String::from_utf8_lossy(&raw[..end]).trim().to_string()
        };
        match block[3] {
            0xfc => model = Some(text()),
            0xff => serial = Some(text()),
            _ => {}
        }
    }
    Some(EdidInfo {
        make: make.trim_matches('@').to_string(),
        model: model
            .filter(|m| !m.is_empty())
            .unwrap_or_else(|| format!("0x{product:04X}")),
        serial: serial.filter(|s| !s.is_empty()).unwrap_or_else(|| {
            if serial_number == 0 {
                String::new()
            } else {
                serial_number.to_string()
            }
        }),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample() -> Vec<u8> {
        let mut e = vec![0u8; 128];
        e[..8].copy_from_slice(&[0x00, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0x00]);
        // "DEL": D=4, E=5, L=12 → 0b0_00100_00101_01100
        let id: u16 = (4 << 10) | (5 << 5) | 12;
        e[8..10].copy_from_slice(&id.to_be_bytes());
        e[10..12].copy_from_slice(&0xa0b1u16.to_le_bytes());
        e[12..16].copy_from_slice(&1234u32.to_le_bytes());
        let name = |e: &mut Vec<u8>, at: usize, tag: u8, s: &str| {
            e[at + 3] = tag;
            let mut text = [b' '; 13];
            text[..s.len()].copy_from_slice(s.as_bytes());
            if s.len() < 13 {
                text[s.len()] = b'\n';
            }
            e[at + 5..at + 18].copy_from_slice(&text);
        };
        name(&mut e, 72, 0xfc, "DELL U2720Q");
        name(&mut e, 90, 0xff, "ABC123");
        e
    }

    #[test]
    fn parses_make_model_serial() {
        let info = parse(&sample()).unwrap();
        assert_eq!(info.make, "DEL");
        assert_eq!(info.model, "DELL U2720Q");
        assert_eq!(info.serial, "ABC123");
    }

    #[test]
    fn falls_back_to_codes() {
        let mut e = sample();
        e[72 + 3] = 0x10;
        e[90 + 3] = 0x10;
        let info = parse(&e).unwrap();
        assert_eq!(info.model, "0xA0B1");
        assert_eq!(info.serial, "1234");
        assert!(parse(&e[..100]).is_none());
        e[0] = 1;
        assert!(parse(&e).is_none());
    }
}
