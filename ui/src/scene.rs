//! Render-agnostic description of a frame: rectangles, text runs and effects.

use crate::geometry::{Color, Rect};

/// How a rectangle instance is shaded.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[repr(u32)]
pub enum RectKind {
    /// Filled rectangle with an optional border and glow.
    Plain = 0,
    /// Beveled key cap (octagonal cut corners).
    KeyCap = 1,
    /// Filled circle inscribed in the rectangle.
    Circle = 2,
    /// Hexagon inscribed in the rectangle (flat top).
    Hexagon = 3,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct RectInstance {
    pub rect: Rect,
    pub fill: Color,
    pub border: Color,
    pub border_width: f32,
    pub radius: f32,
    /// 0..=1 glow intensity applied to the border.
    pub glow: f32,
    pub kind: RectKind,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub enum Align {
    #[default]
    Left,
    Center,
    Right,
}

#[derive(Clone, Debug, PartialEq)]
pub struct TextSpan {
    pub text: String,
    pub color: Color,
    pub bold: bool,
    pub italic: bool,
}

#[derive(Clone, Debug, PartialEq)]
pub struct TextSpec {
    /// Bounds in logical pixels; text is clipped to this rectangle.
    pub bounds: Rect,
    pub size: f32,
    pub line_height: f32,
    pub spans: Vec<TextSpan>,
    pub align: Align,
    pub wrap: bool,
    pub mono: bool,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Scanlines {
    pub color: Color,
    pub intensity: f32,
}

/// One frame's worth of drawing commands in logical pixels.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Scene {
    pub width: f32,
    pub height: f32,
    pub clear: Color,
    pub rects: Vec<RectInstance>,
    pub texts: Vec<TextSpec>,
    pub scanlines: Option<Scanlines>,
}

impl Scene {
    pub fn new(width: f32, height: f32, clear: Color) -> Self {
        Self { width, height, clear, ..Default::default() }
    }

    pub fn fill(&mut self, rect: Rect, color: Color) {
        self.rects.push(RectInstance {
            rect,
            fill: color,
            border: [0.0; 4],
            border_width: 0.0,
            radius: 0.0,
            glow: 0.0,
            kind: RectKind::Plain,
        });
    }

    pub fn fill_rounded(&mut self, rect: Rect, color: Color, radius: f32) {
        self.rects.push(RectInstance {
            rect,
            fill: color,
            border: [0.0; 4],
            border_width: 0.0,
            radius,
            glow: 0.0,
            kind: RectKind::Plain,
        });
    }

    pub fn stroke(&mut self, rect: Rect, color: Color, width: f32) {
        self.rects.push(RectInstance {
            rect,
            fill: [0.0; 4],
            border: color,
            border_width: width,
            radius: 0.0,
            glow: 0.0,
            kind: RectKind::Plain,
        });
    }

    pub fn panel(&mut self, rect: Rect, fill: Color, border: Color, width: f32, glow: f32) {
        self.rects.push(RectInstance {
            rect,
            fill,
            border,
            border_width: width,
            radius: 0.0,
            glow,
            kind: RectKind::Plain,
        });
    }

    pub fn shape(&mut self, kind: RectKind, rect: Rect, fill: Color, border: Color, width: f32) {
        self.rects.push(RectInstance { rect, fill, border, border_width: width, radius: 0.0, glow: 0.0, kind });
    }

    pub fn hline(&mut self, x: f32, y: f32, w: f32, color: Color) {
        self.fill(Rect::new(x, y, w, 1.0), color);
    }

    pub fn vline(&mut self, x: f32, y: f32, h: f32, color: Color) {
        self.fill(Rect::new(x, y, 1.0, h), color);
    }

    pub fn text(&mut self, bounds: Rect, size: f32, color: Color, text: impl Into<String>) {
        self.texts.push(TextSpec {
            bounds,
            size,
            line_height: (size * 1.3).round(),
            spans: vec![TextSpan { text: text.into(), color, bold: false, italic: false }],
            align: Align::Left,
            wrap: false,
            mono: true,
        });
    }

    pub fn text_aligned(&mut self, bounds: Rect, size: f32, color: Color, align: Align, text: impl Into<String>) {
        self.texts.push(TextSpec {
            bounds,
            size,
            line_height: (size * 1.3).round(),
            spans: vec![TextSpan { text: text.into(), color, bold: false, italic: false }],
            align,
            wrap: false,
            mono: true,
        });
    }

    pub fn text_bold(&mut self, bounds: Rect, size: f32, color: Color, align: Align, text: impl Into<String>) {
        self.texts.push(TextSpec {
            bounds,
            size,
            line_height: (size * 1.3).round(),
            spans: vec![TextSpan { text: text.into(), color, bold: true, italic: false }],
            align,
            wrap: false,
            mono: true,
        });
    }

    pub fn paragraph(&mut self, bounds: Rect, size: f32, color: Color, text: impl Into<String>) {
        self.texts.push(TextSpec {
            bounds,
            size,
            line_height: (size * 1.4).round(),
            spans: vec![TextSpan { text: text.into(), color, bold: false, italic: false }],
            align: Align::Left,
            wrap: true,
            mono: true,
        });
    }

    pub fn spans(&mut self, bounds: Rect, size: f32, line_height: f32, spans: Vec<TextSpan>) {
        self.texts.push(TextSpec { bounds, size, line_height, spans, align: Align::Left, wrap: false, mono: true });
    }

    /// Horizontal progress bar.
    pub fn bar(&mut self, rect: Rect, frac: f32, track: Color, fill: Color) {
        self.fill(rect, track);
        let w = (rect.w * frac.clamp(0.0, 1.0)).round();
        if w > 0.0 {
            self.fill(Rect::new(rect.x, rect.y, w, rect.h), fill);
        }
    }

    /// Sparkline of `values` (0..=1) drawn as vertical bars.
    pub fn sparkline(&mut self, rect: Rect, values: &[f32], color: Color, track: Color) {
        self.fill(rect, track);
        if values.is_empty() || rect.w <= 0.0 {
            return;
        }
        let n = values.len() as f32;
        let bw = rect.w / n;
        for (i, v) in values.iter().enumerate() {
            let h = (rect.h * v.clamp(0.0, 1.0)).round();
            if h > 0.0 {
                let x = rect.x + i as f32 * bw;
                self.fill(Rect::new(x, rect.bottom() - h, (bw - 1.0).max(1.0), h), color);
            }
        }
    }
}
