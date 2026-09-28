//! Keyboard → PTY byte mapping (xterm-compatible, with application cursor/keypad modes).

#![allow(non_upper_case_globals)]

use xkbcommon::xkb::keysyms::*;

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Modifiers {
    pub ctrl: bool,
    pub alt: bool,
    pub shift: bool,
    pub logo: bool,
}

impl Modifiers {
    /// xterm modifier parameter (1 = none, 2 = shift, 3 = alt, 5 = ctrl, combinations add).
    fn xterm_param(&self) -> u8 {
        1 + (self.shift as u8) + (self.alt as u8) * 2 + (self.ctrl as u8) * 4
    }

    fn any_modifier(&self) -> bool {
        self.ctrl || self.alt || self.shift
    }
}

/// Terminal modes that change key encoding.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct KeyModes {
    pub app_cursor: bool,
    pub app_keypad: bool,
}

fn csi_or_ss3(final_byte: u8, mods: Modifiers, app: bool, csi_number: Option<u8>) -> Vec<u8> {
    if mods.any_modifier() {
        match csi_number {
            Some(n) => format!("\x1b[{n};{}~", mods.xterm_param()).into_bytes(),
            None => format!("\x1b[1;{}{}", mods.xterm_param(), final_byte as char).into_bytes(),
        }
    } else {
        match csi_number {
            Some(n) => format!("\x1b[{n}~").into_bytes(),
            None if app => vec![0x1b, b'O', final_byte],
            None => vec![0x1b, b'[', final_byte],
        }
    }
}

/// Encode a key into PTY bytes. Returns `None` for keys that produce no input
/// (bare modifiers, unhandled specials).
pub fn key_to_bytes(keysym: u32, text: Option<&str>, mods: Modifiers, modes: KeyModes) -> Option<Vec<u8>> {
    let app = modes.app_cursor;
    let bytes = match keysym {
        KEY_Return | KEY_KP_Enter | KEY_ISO_Enter => {
            if mods.alt {
                b"\x1b\r".to_vec()
            } else {
                b"\r".to_vec()
            }
        }
        KEY_BackSpace => {
            if mods.ctrl {
                b"\x08".to_vec()
            } else if mods.alt {
                b"\x1b\x7f".to_vec()
            } else {
                b"\x7f".to_vec()
            }
        }
        KEY_Tab | KEY_ISO_Left_Tab => {
            if mods.shift {
                b"\x1b[Z".to_vec()
            } else {
                b"\t".to_vec()
            }
        }
        KEY_Escape => b"\x1b".to_vec(),
        KEY_Up | KEY_KP_Up => csi_or_ss3(b'A', mods, app, None),
        KEY_Down | KEY_KP_Down => csi_or_ss3(b'B', mods, app, None),
        KEY_Right | KEY_KP_Right => csi_or_ss3(b'C', mods, app, None),
        KEY_Left | KEY_KP_Left => csi_or_ss3(b'D', mods, app, None),
        KEY_Home | KEY_KP_Home => csi_or_ss3(b'H', mods, app, None),
        KEY_End | KEY_KP_End => csi_or_ss3(b'F', mods, app, None),
        KEY_Insert | KEY_KP_Insert => csi_or_ss3(0, mods, app, Some(2)),
        KEY_Delete | KEY_KP_Delete => csi_or_ss3(0, mods, app, Some(3)),
        KEY_Page_Up | KEY_KP_Page_Up => csi_or_ss3(0, mods, app, Some(5)),
        KEY_Page_Down | KEY_KP_Page_Down => csi_or_ss3(0, mods, app, Some(6)),
        KEY_F1 => fkey(b'P', 11, mods),
        KEY_F2 => fkey(b'Q', 12, mods),
        KEY_F3 => fkey(b'R', 13, mods),
        KEY_F4 => fkey(b'S', 14, mods),
        KEY_F5 => csi_or_ss3(0, mods, app, Some(15)),
        KEY_F6 => csi_or_ss3(0, mods, app, Some(17)),
        KEY_F7 => csi_or_ss3(0, mods, app, Some(18)),
        KEY_F8 => csi_or_ss3(0, mods, app, Some(19)),
        KEY_F9 => csi_or_ss3(0, mods, app, Some(20)),
        KEY_F10 => csi_or_ss3(0, mods, app, Some(21)),
        KEY_F11 => csi_or_ss3(0, mods, app, Some(23)),
        KEY_F12 => csi_or_ss3(0, mods, app, Some(24)),
        _ => {
            if mods.ctrl {
                return ctrl_bytes(keysym, text).map(|b| alt_prefix(b, mods));
            }
            let text = text?;
            if text.is_empty() {
                return None;
            }
            return Some(alt_prefix(text.as_bytes().to_vec(), mods));
        }
    };
    Some(bytes)
}

fn fkey(ss3: u8, csi: u8, mods: Modifiers) -> Vec<u8> {
    if mods.any_modifier() {
        format!("\x1b[{csi};{}~", mods.xterm_param()).into_bytes()
    } else {
        vec![0x1b, b'O', ss3]
    }
}

fn ctrl_bytes(keysym: u32, text: Option<&str>) -> Option<Vec<u8>> {
    let byte = match keysym {
        KEY_at | KEY_2 | KEY_space | KEY_KP_Space => 0x00,
        KEY_bracketleft | KEY_3 => 0x1b,
        KEY_backslash | KEY_4 => 0x1c,
        KEY_bracketright | KEY_5 => 0x1d,
        KEY_asciicircum | KEY_6 => 0x1e,
        KEY_underscore | KEY_7 | KEY_slash | KEY_question => 0x1f,
        KEY_8 => 0x7f,
        _ => {
            let ch = char::from_u32(keysym).or_else(|| text.and_then(|t| t.chars().next()))?;
            let upper = ch.to_ascii_uppercase();
            if upper.is_ascii_uppercase() {
                (upper as u8) & 0x1f
            } else {
                return text.map(|t| t.as_bytes().to_vec()).filter(|t| !t.is_empty());
            }
        }
    };
    Some(vec![byte])
}

fn alt_prefix(mut bytes: Vec<u8>, mods: Modifiers) -> Vec<u8> {
    if mods.alt {
        bytes.insert(0, 0x1b);
    }
    bytes
}

/// Wrap pasted text according to bracketed-paste mode and normalise newlines.
pub fn encode_paste(text: &str, bracketed: bool) -> Vec<u8> {
    let normalized = text.replace("\r\n", "\r").replace('\n', "\r");
    if bracketed {
        let mut out = b"\x1b[200~".to_vec();
        // Strip any embedded end-of-paste sequence to prevent injection.
        out.extend_from_slice(normalized.replace("\x1b[201~", "").as_bytes());
        out.extend_from_slice(b"\x1b[201~");
        out
    } else {
        normalized.into_bytes()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn maps_basic_keys() {
        let m = Modifiers::default();
        assert_eq!(key_to_bytes(KEY_Return, None, m, KeyModes::default()), Some(b"\r".to_vec()));
        assert_eq!(key_to_bytes(KEY_c, Some("c"), Modifiers { ctrl: true, ..m }, KeyModes::default()), Some(vec![0x03]));
        assert_eq!(key_to_bytes(KEY_a, Some("a"), m, KeyModes::default()), Some(b"a".to_vec()));
        assert_eq!(key_to_bytes(KEY_a, Some("a"), Modifiers { alt: true, ..m }, KeyModes::default()), Some(b"\x1ba".to_vec()));
    }

    #[test]
    fn arrows_respect_application_mode_and_modifiers() {
        let m = Modifiers::default();
        assert_eq!(key_to_bytes(KEY_Up, None, m, KeyModes::default()), Some(b"\x1b[A".to_vec()));
        assert_eq!(key_to_bytes(KEY_Up, None, m, KeyModes { app_cursor: true, app_keypad: false }), Some(b"\x1bOA".to_vec()));
        assert_eq!(key_to_bytes(KEY_Up, None, Modifiers { ctrl: true, ..m }, KeyModes::default()), Some(b"\x1b[1;5A".to_vec()));
        assert_eq!(key_to_bytes(KEY_Delete, None, Modifiers { shift: true, ..m }, KeyModes::default()), Some(b"\x1b[3;2~".to_vec()));
        assert_eq!(key_to_bytes(KEY_F5, None, m, KeyModes::default()), Some(b"\x1b[15~".to_vec()));
    }

    #[test]
    fn paste_is_bracketed_and_sanitised() {
        assert_eq!(encode_paste("a\nb", false), b"a\rb".to_vec());
        assert_eq!(encode_paste("x\x1b[201~y", true), b"\x1b[200~xy\x1b[201~".to_vec());
    }
}
