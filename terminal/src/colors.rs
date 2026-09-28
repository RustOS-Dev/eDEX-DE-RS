//! Map alacritty colours onto the theme palette.

use alacritty_terminal::{
    term::color::Colors,
    vte::ansi::{Color, NamedColor, Rgb},
};
use ui::geometry::Color as UiColor;

/// Theme-derived palette used to resolve terminal colours.
#[derive(Clone, Debug, PartialEq)]
pub struct Palette {
    pub fg: UiColor,
    pub bg: UiColor,
    pub cursor: UiColor,
    pub ansi: [UiColor; 16],
}

fn rgb(c: Rgb) -> UiColor {
    [c.r as f32 / 255.0, c.g as f32 / 255.0, c.b as f32 / 255.0, 1.0]
}

fn dim(c: UiColor) -> UiColor {
    [c[0] * 0.66, c[1] * 0.66, c[2] * 0.66, c[3]]
}

fn bright(c: UiColor) -> UiColor {
    [(c[0] * 1.2).min(1.0), (c[1] * 1.2).min(1.0), (c[2] * 1.2).min(1.0), c[3]]
}

impl Palette {
    /// Resolve a cell colour, honouring OSC overrides stored in the terminal's `Colors`.
    pub fn resolve(&self, color: Color, overrides: &Colors, bold: bool) -> UiColor {
        match color {
            Color::Spec(c) => rgb(c),
            Color::Indexed(i) => {
                if let Some(c) = overrides[i as usize] {
                    return rgb(c);
                }
                self.indexed(i, bold)
            }
            Color::Named(n) => {
                if let Some(c) = overrides[n] {
                    return rgb(c);
                }
                self.named(n, bold)
            }
        }
    }

    fn indexed(&self, i: u8, bold: bool) -> UiColor {
        match i {
            0..=7 if bold => self.ansi[i as usize + 8],
            0..=15 => self.ansi[i as usize],
            16..=231 => {
                let i = i - 16;
                let r = i / 36;
                let g = (i % 36) / 6;
                let b = i % 6;
                let f = |v: u8| if v == 0 { 0.0 } else { (55.0 + v as f32 * 40.0) / 255.0 };
                [f(r), f(g), f(b), 1.0]
            }
            _ => {
                let v = (8 + (i as u16 - 232) * 10) as f32 / 255.0;
                [v, v, v, 1.0]
            }
        }
    }

    fn named(&self, n: NamedColor, bold: bool) -> UiColor {
        let idx = n as usize;
        match n {
            NamedColor::Foreground | NamedColor::BrightForeground => self.fg,
            NamedColor::Background => self.bg,
            NamedColor::Cursor => self.cursor,
            NamedColor::DimForeground => dim(self.fg),
            NamedColor::DimBlack
            | NamedColor::DimRed
            | NamedColor::DimGreen
            | NamedColor::DimYellow
            | NamedColor::DimBlue
            | NamedColor::DimMagenta
            | NamedColor::DimCyan
            | NamedColor::DimWhite => dim(self.ansi[idx - NamedColor::DimBlack as usize]),
            _ if idx < 8 && bold => bright(self.ansi[idx + 8]),
            _ if idx < 16 => self.ansi[idx],
            _ => self.fg,
        }
    }
}
