//! The on-screen hex keyboard: layout, hit rectangles and highlight state.

use std::collections::HashSet;

use crate::{
    geometry::Rect,
    layout::{KEYBOARD_PAD, KEYBOARD_ROWS, KEY_GAP},
};

/// What pressing a key on the on-screen keyboard should do.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum KeyAction {
    Char(char),
    Escape,
    Tab,
    Backspace,
    Enter,
    Space,
    Shift,
    Ctrl,
    Alt,
    Super,
    CapsLock,
    Up,
    Down,
    Left,
    Right,
}

#[derive(Clone, Copy, Debug)]
pub struct KeyDef {
    pub label: &'static str,
    pub shifted: &'static str,
    pub units: f32,
    pub action: KeyAction,
}

const fn key(label: &'static str, shifted: &'static str, units: f32, action: KeyAction) -> KeyDef {
    KeyDef { label, shifted, units, action }
}

const fn ch(label: &'static str, shifted: &'static str, c: char) -> KeyDef {
    KeyDef { label, shifted, units: 1.0, action: KeyAction::Char(c) }
}

pub const ROWS: [&[KeyDef]; KEYBOARD_ROWS] = [
    &[
        key("ESC", "ESC", 1.0, KeyAction::Escape),
        ch("1", "!", '1'), ch("2", "@", '2'), ch("3", "#", '3'), ch("4", "$", '4'), ch("5", "%", '5'),
        ch("6", "^", '6'), ch("7", "&", '7'), ch("8", "*", '8'), ch("9", "(", '9'), ch("0", ")", '0'),
        ch("-", "_", '-'), ch("=", "+", '='),
        key("BKSP", "BKSP", 2.0, KeyAction::Backspace),
    ],
    &[
        key("TAB", "TAB", 1.5, KeyAction::Tab),
        ch("q", "Q", 'q'), ch("w", "W", 'w'), ch("e", "E", 'e'), ch("r", "R", 'r'), ch("t", "T", 't'),
        ch("y", "Y", 'y'), ch("u", "U", 'u'), ch("i", "I", 'i'), ch("o", "O", 'o'), ch("p", "P", 'p'),
        ch("[", "{", '['), ch("]", "}", ']'),
        key("\\", "|", 1.5, KeyAction::Char('\\')),
    ],
    &[
        key("CAPS", "CAPS", 1.75, KeyAction::CapsLock),
        ch("a", "A", 'a'), ch("s", "S", 's'), ch("d", "D", 'd'), ch("f", "F", 'f'), ch("g", "G", 'g'),
        ch("h", "H", 'h'), ch("j", "J", 'j'), ch("k", "K", 'k'), ch("l", "L", 'l'),
        ch(";", ":", ';'), ch("'", "\"", '\''),
        key("ENTER", "ENTER", 2.25, KeyAction::Enter),
    ],
    &[
        key("SHIFT", "SHIFT", 2.25, KeyAction::Shift),
        ch("z", "Z", 'z'), ch("x", "X", 'x'), ch("c", "C", 'c'), ch("v", "V", 'v'), ch("b", "B", 'b'),
        ch("n", "N", 'n'), ch("m", "M", 'm'), ch(",", "<", ','), ch(".", ">", '.'), ch("/", "?", '/'),
        key("SHIFT", "SHIFT", 2.75, KeyAction::Shift),
    ],
    &[
        key("CTRL", "CTRL", 1.5, KeyAction::Ctrl),
        key("SUPER", "SUPER", 1.25, KeyAction::Super),
        key("ALT", "ALT", 1.25, KeyAction::Alt),
        key("", "", 6.0, KeyAction::Space),
        key("ALT", "ALT", 1.25, KeyAction::Alt),
        key("←", "←", 1.0, KeyAction::Left),
        key("↓", "↓", 1.0, KeyAction::Down),
        key("↑", "↑", 1.0, KeyAction::Up),
        key("→", "→", 1.0, KeyAction::Right),
        key("CTRL", "CTRL", 1.5, KeyAction::Ctrl),
    ],
];

/// Highlight state of the on-screen keyboard.
#[derive(Clone, Debug, Default)]
pub struct KeyboardState {
    /// Keys currently held (physical or on-screen).
    pub pressed: HashSet<(usize, usize)>,
    pub hover: Option<(usize, usize)>,
    pub shift: bool,
    pub ctrl: bool,
    pub alt: bool,
    pub logo: bool,
    pub caps_lock: bool,
    /// Sticky modifier armed by clicking an on-screen modifier key.
    pub sticky_shift: bool,
    pub sticky_ctrl: bool,
    pub sticky_alt: bool,
}

impl KeyboardState {
    pub fn is_pressed(&self, row: usize, col: usize) -> bool {
        self.pressed.contains(&(row, col))
    }

    /// Whether a modifier key at the given position should render as active.
    pub fn modifier_active(&self, action: KeyAction) -> bool {
        match action {
            KeyAction::Shift => self.shift || self.sticky_shift,
            KeyAction::Ctrl => self.ctrl || self.sticky_ctrl,
            KeyAction::Alt => self.alt || self.sticky_alt,
            KeyAction::Super => self.logo,
            KeyAction::CapsLock => self.caps_lock,
            _ => false,
        }
    }
}

/// Rectangle of a key inside the keyboard panel.
pub fn key_rect(panel: Rect, key_h: f32, row: usize, col: usize) -> Rect {
    let row_defs = ROWS[row];
    let total_units: f32 = row_defs.iter().map(|k| k.units).sum();
    let inner_w = (panel.w - KEYBOARD_PAD * 2.0).max(1.0);
    let unit_w = ((inner_w - KEY_GAP * (row_defs.len() as f32 - 1.0)) / total_units).max(8.0);
    let x_units: f32 = row_defs.iter().take(col).map(|k| k.units).sum();
    let x = panel.x + KEYBOARD_PAD + x_units * unit_w + col as f32 * KEY_GAP;
    let y = panel.y + KEYBOARD_PAD + row as f32 * (key_h + KEY_GAP);
    let w = row_defs[col].units * unit_w;
    Rect::new(x.round(), y.round(), w.round(), key_h)
}

/// Find the on-screen key matching a typed character or named action.
pub fn find_key(action: KeyAction) -> Option<(usize, usize)> {
    for (r, row) in ROWS.iter().enumerate() {
        for (c, k) in row.iter().enumerate() {
            let same = match (k.action, action) {
                (KeyAction::Char(a), KeyAction::Char(b)) => a.eq_ignore_ascii_case(&b),
                (a, b) => a == b,
            };
            if same {
                return Some((r, c));
            }
        }
    }
    None
}

/// Map a keysym (xkb value) to a key action for highlighting.
pub fn action_for_keysym(keysym: u32, text: Option<&str>) -> Option<KeyAction> {
    match keysym {
        0xff1b => Some(KeyAction::Escape),
        0xff09 | 0xfe20 => Some(KeyAction::Tab),
        0xff08 => Some(KeyAction::Backspace),
        0xff0d | 0xff8d => Some(KeyAction::Enter),
        0x20 => Some(KeyAction::Space),
        0xffe1 | 0xffe2 => Some(KeyAction::Shift),
        0xffe3 | 0xffe4 => Some(KeyAction::Ctrl),
        0xffe9 | 0xffea | 0xfe03 => Some(KeyAction::Alt),
        0xffeb | 0xffec => Some(KeyAction::Super),
        0xffe5 => Some(KeyAction::CapsLock),
        0xff52 => Some(KeyAction::Up),
        0xff54 => Some(KeyAction::Down),
        0xff51 => Some(KeyAction::Left),
        0xff53 => Some(KeyAction::Right),
        _ => {
            let c = text.and_then(|t| t.chars().next()).or_else(|| char::from_u32(keysym))?;
            let base = unshift(c);
            Some(KeyAction::Char(base))
        }
    }
}

fn unshift(c: char) -> char {
    match c {
        '!' => '1', '@' => '2', '#' => '3', '$' => '4', '%' => '5', '^' => '6', '&' => '7', '*' => '8',
        '(' => '9', ')' => '0', '_' => '-', '+' => '=', '{' => '[', '}' => ']', '|' => '\\', ':' => ';',
        '"' => '\'', '<' => ',', '>' => '.', '?' => '/', '~' => '`',
        other => other.to_ascii_lowercase(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rows_have_consistent_units() {
        for row in ROWS {
            let units: f32 = row.iter().map(|k| k.units).sum();
            assert!((14.0..=17.0).contains(&units), "row units {units}");
        }
    }

    #[test]
    fn keys_do_not_overlap() {
        let panel = Rect::new(0.0, 0.0, 1920.0, 220.0);
        for r in 0..KEYBOARD_ROWS {
            let mut last_right = -1.0;
            for c in 0..ROWS[r].len() {
                let rect = key_rect(panel, 32.0, r, c);
                assert!(rect.x >= last_right, "row {r} col {c}");
                last_right = rect.right();
            }
            assert!(last_right <= panel.w);
        }
    }

    #[test]
    fn finds_keys_for_keysyms() {
        assert_eq!(action_for_keysym(0x41, Some("A")), Some(KeyAction::Char('a')));
        assert_eq!(find_key(KeyAction::Char('A')), Some((2, 1)));
        assert_eq!(action_for_keysym(0xff0d, None), Some(KeyAction::Enter));
        assert_eq!(find_key(KeyAction::Enter), Some((2, 12)));
    }
}
