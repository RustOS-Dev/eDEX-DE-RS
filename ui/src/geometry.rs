//! Basic geometry and colour helpers (logical pixel coordinates).

/// RGBA colour with components in 0..=1 (sRGB, straight alpha).
pub type Color = [f32; 4];

#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Rect {
    pub x: f32,
    pub y: f32,
    pub w: f32,
    pub h: f32,
}

impl Rect {
    pub const fn new(x: f32, y: f32, w: f32, h: f32) -> Self {
        Self { x, y, w, h }
    }

    pub fn right(&self) -> f32 {
        self.x + self.w
    }

    pub fn bottom(&self) -> f32 {
        self.y + self.h
    }

    pub fn contains(&self, px: f32, py: f32) -> bool {
        px >= self.x && py >= self.y && px < self.right() && py < self.bottom()
    }

    pub fn inset(&self, d: f32) -> Rect {
        Rect::new(
            self.x + d,
            self.y + d,
            (self.w - 2.0 * d).max(0.0),
            (self.h - 2.0 * d).max(0.0),
        )
    }

    pub fn inset_xy(&self, dx: f32, dy: f32) -> Rect {
        Rect::new(
            self.x + dx,
            self.y + dy,
            (self.w - 2.0 * dx).max(0.0),
            (self.h - 2.0 * dy).max(0.0),
        )
    }

    /// Sub-rectangle at the top of this one.
    pub fn top(&self, h: f32) -> Rect {
        Rect::new(self.x, self.y, self.w, h.min(self.h))
    }

    /// Sub-rectangle below `offset` from the top.
    pub fn below(&self, offset: f32) -> Rect {
        Rect::new(self.x, self.y + offset, self.w, (self.h - offset).max(0.0))
    }

    pub fn centered(&self, w: f32, h: f32) -> Rect {
        Rect::new(
            self.x + (self.w - w) / 2.0,
            self.y + (self.h - h) / 2.0,
            w,
            h,
        )
    }

    pub fn round(&self) -> Rect {
        Rect::new(
            self.x.round(),
            self.y.round(),
            self.w.round(),
            self.h.round(),
        )
    }

    pub fn is_empty(&self) -> bool {
        self.w <= 0.0 || self.h <= 0.0
    }
}

pub fn with_alpha(c: Color, a: f32) -> Color {
    [c[0], c[1], c[2], a]
}

pub fn mix(a: Color, b: Color, t: f32) -> Color {
    let t = t.clamp(0.0, 1.0);
    [
        a[0] + (b[0] - a[0]) * t,
        a[1] + (b[1] - a[1]) * t,
        a[2] + (b[2] - a[2]) * t,
        a[3] + (b[3] - a[3]) * t,
    ]
}

pub const TRANSPARENT: Color = [0.0, 0.0, 0.0, 0.0];

/// Parse `#rrggbb` or `#rrggbbaa`.
pub fn parse_color(hex: &str) -> Option<Color> {
    let hex = hex.trim().strip_prefix('#')?;
    let (r, g, b, a) = match hex.len() {
        6 => (
            u8::from_str_radix(&hex[0..2], 16).ok()?,
            u8::from_str_radix(&hex[2..4], 16).ok()?,
            u8::from_str_radix(&hex[4..6], 16).ok()?,
            255u8,
        ),
        8 => (
            u8::from_str_radix(&hex[0..2], 16).ok()?,
            u8::from_str_radix(&hex[2..4], 16).ok()?,
            u8::from_str_radix(&hex[4..6], 16).ok()?,
            u8::from_str_radix(&hex[6..8], 16).ok()?,
        ),
        _ => return None,
    };
    Some([
        r as f32 / 255.0,
        g as f32 / 255.0,
        b as f32 / 255.0,
        a as f32 / 255.0,
    ])
}

pub fn format_color(c: Color) -> String {
    format!(
        "#{:02x}{:02x}{:02x}",
        (c[0] * 255.0).round() as u8,
        (c[1] * 255.0).round() as u8,
        (c[2] * 255.0).round() as u8
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_hex_colors() {
        assert_eq!(parse_color("#00e5ff"), Some([0.0, 229.0 / 255.0, 1.0, 1.0]));
        assert_eq!(
            parse_color("#ff000080").map(|c| (c[3] * 255.0).round() as u8),
            Some(128)
        );
        assert_eq!(parse_color("nope"), None);
    }

    #[test]
    fn rect_contains_and_inset() {
        let r = Rect::new(10.0, 10.0, 100.0, 50.0);
        assert!(r.contains(10.0, 10.0));
        assert!(!r.contains(110.0, 10.0));
        assert_eq!(r.inset(5.0), Rect::new(15.0, 15.0, 90.0, 40.0));
    }
}
