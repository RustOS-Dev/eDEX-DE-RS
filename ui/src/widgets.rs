//! Drawing helpers shared by panels and overlays.

use crate::{
    geometry::{mix, with_alpha, Color, Rect},
    hit::{HitMap, HitTarget},
    layout::Metrics,
    scene::{Align, RectKind, Scene},
    theme::Theme,
};

/// Everything a drawing function needs.
pub struct Ctx<'a> {
    pub scene: &'a mut Scene,
    pub hits: &'a mut HitMap,
    pub theme: &'a Theme,
    pub metrics: &'a Metrics,
    pub pulse: f32,
}

impl Ctx<'_> {
    pub fn font(&self) -> f32 {
        self.metrics.ui_font
    }

    pub fn small(&self) -> f32 {
        (self.metrics.ui_font * 0.85).round()
    }

    pub fn line(&self) -> f32 {
        self.metrics.line
    }

    /// Panel frame with the eDEX glowing border.
    pub fn frame(&mut self, rect: Rect, title: Option<&str>) -> Rect {
        let t = self.theme;
        let glow = t.glow * (0.5 + 0.5 * self.pulse);
        self.scene.panel(rect, t.panel_bg, t.border, 1.0, glow);
        // corner accents
        let c = 10.0;
        let accent = t.border;
        for (x, y, dx, dy) in [
            (rect.x, rect.y, 1.0, 1.0),
            (rect.right() - c, rect.y, 1.0, 1.0),
            (rect.x, rect.bottom() - 2.0, 1.0, -1.0),
            (rect.right() - c, rect.bottom() - 2.0, 1.0, -1.0),
        ] {
            let _ = (dx, dy);
            self.scene.fill(Rect::new(x, y, c, 2.0), accent);
        }
        for (x, y) in [(rect.x, rect.y), (rect.right() - 2.0, rect.y), (rect.x, rect.bottom() - c), (rect.right() - 2.0, rect.bottom() - c)] {
            self.scene.fill(Rect::new(x, y, 2.0, c), accent);
        }
        if let Some(title) = title {
            let h = self.line();
            let bar = Rect::new(rect.x + 1.0, rect.y + 1.0, rect.w - 2.0, h);
            self.scene.fill(bar, with_alpha(t.border, 0.12));
            self.scene.text_bold(Rect::new(bar.x + 8.0, bar.y, bar.w - 16.0, h), self.small(), t.border, Align::Left, title);
            self.scene.hline(bar.x, bar.bottom(), bar.w, with_alpha(t.border, 0.5));
            Rect::new(rect.x + 6.0, bar.bottom() + 4.0, rect.w - 12.0, rect.bottom() - bar.bottom() - 10.0)
        } else {
            rect.inset(6.0)
        }
    }

    pub fn label(&mut self, rect: Rect, text: &str, color: Color) {
        let size = self.font();
        self.scene.text(rect, size, color, text);
    }

    pub fn label_small(&mut self, rect: Rect, text: &str, color: Color) {
        let size = self.small();
        self.scene.text(rect, size, color, text);
    }

    pub fn label_right(&mut self, rect: Rect, text: &str, color: Color) {
        let size = self.font();
        self.scene.text_aligned(rect, size, color, Align::Right, text);
    }

    pub fn label_center(&mut self, rect: Rect, text: &str, color: Color) {
        let size = self.font();
        self.scene.text_aligned(rect, size, color, Align::Center, text);
    }

    pub fn heading(&mut self, rect: Rect, text: &str) {
        let size = (self.font() * 1.15).round();
        let color = self.theme.border;
        self.scene.text_bold(rect, size, color, Align::Left, text);
    }

    /// A clickable button; returns its rect.
    pub fn button(&mut self, rect: Rect, text: &str, target: HitTarget, focused: bool, enabled: bool) -> Rect {
        let t = self.theme;
        let (fill, border, fg) = if !enabled {
            (with_alpha(t.panel_bg, 0.6), with_alpha(t.border, 0.25), t.text_dim)
        } else if focused {
            (with_alpha(t.border, 0.25), t.border, t.text_primary)
        } else {
            (with_alpha(t.border, 0.08), with_alpha(t.border, 0.6), t.text_primary)
        };
        self.scene.panel(rect, fill, border, 1.0, if focused { 0.8 } else { 0.0 });
        self.label_center(Rect::new(rect.x, rect.y + (rect.h - self.line()) / 2.0, rect.w, self.line()), text, fg);
        if enabled {
            self.hits.push(rect, target);
        }
        rect
    }

    pub fn toggle(&mut self, rect: Rect, on: bool, target: HitTarget, focused: bool, enabled: bool) {
        let t = self.theme;
        let track = if on { with_alpha(t.accent, if enabled { 0.6 } else { 0.25 }) } else { with_alpha(t.text_secondary, 0.25) };
        let border = if focused { t.border } else { with_alpha(t.border, 0.5) };
        self.scene.fill_rounded(rect, track, rect.h / 2.0);
        self.scene.stroke(rect, border, 1.0);
        let knob = rect.h - 4.0;
        let x = if on { rect.right() - knob - 2.0 } else { rect.x + 2.0 };
        let knob_color = if on { t.accent } else { t.text_secondary };
        self.scene.shape(RectKind::Circle, Rect::new(x, rect.y + 2.0, knob, knob), knob_color, [0.0; 4], 0.0);
        if enabled {
            self.hits.push(rect, target);
        }
    }

    pub fn slider(&mut self, rect: Rect, frac: f32, target: HitTarget, focused: bool) {
        let t = self.theme;
        let track = Rect::new(rect.x, rect.y + rect.h / 2.0 - 2.0, rect.w, 4.0);
        self.scene.fill(track, with_alpha(t.text_secondary, 0.25));
        let fill_w = (track.w * frac.clamp(0.0, 1.0)).round();
        self.scene.fill(Rect::new(track.x, track.y, fill_w, track.h), t.border);
        let knob = 12.0;
        let kx = (track.x + fill_w - knob / 2.0).clamp(track.x, track.right() - knob);
        let knob_color = if focused { t.text_primary } else { t.border };
        self.scene.shape(RectKind::Circle, Rect::new(kx, rect.y + rect.h / 2.0 - knob / 2.0, knob, knob), knob_color, t.background, 1.0);
        self.hits.push(rect, target);
    }

    pub fn text_input(&mut self, rect: Rect, value: &str, placeholder: &str, editing: bool, secret: bool, target: HitTarget) {
        let t = self.theme;
        let border = if editing { t.border } else { with_alpha(t.border, 0.5) };
        self.scene.panel(rect, with_alpha(t.background, 0.8), border, 1.0, if editing { 0.6 } else { 0.0 });
        let shown: String = if secret { "•".repeat(value.chars().count()) } else { value.to_string() };
        let text_rect = Rect::new(rect.x + 8.0, rect.y + (rect.h - self.line()) / 2.0, rect.w - 16.0, self.line());
        if shown.is_empty() && !editing {
            self.label(text_rect, placeholder, t.text_dim);
        } else {
            let caret = if editing && (self.pulse * 4.0) as i32 % 2 == 0 { "▏" } else { " " };
            self.label(text_rect, &format!("{shown}{caret}"), t.text_primary);
        }
        self.hits.push(rect, target);
    }

    /// Row background for list items.
    pub fn row(&mut self, rect: Rect, selected: bool, target: HitTarget) {
        let t = self.theme;
        if selected {
            self.scene.fill(rect, with_alpha(t.border, 0.18));
            self.scene.fill(Rect::new(rect.x, rect.y, 2.0, rect.h), t.border);
        }
        self.hits.push(rect, target);
    }

    pub fn chip(&mut self, rect: Rect, text: &str, active: bool, target: HitTarget) {
        let t = self.theme;
        let (fill, fg) = if active { (t.border, t.background) } else { (with_alpha(t.border, 0.12), t.text_secondary) };
        self.scene.fill_rounded(rect, fill, 3.0);
        self.label_center(Rect::new(rect.x, rect.y + (rect.h - self.line()) / 2.0, rect.w, self.line()), text, fg);
        self.hits.push(rect, target);
    }

    pub fn meter(&mut self, rect: Rect, frac: f32, color: Color) {
        let t = self.theme;
        let c = if frac > 0.9 { t.error } else if frac > 0.7 { t.warning } else { color };
        self.scene.bar(rect, frac, with_alpha(t.text_secondary, 0.18), c);
    }

    /// Dim backdrop for overlays.
    pub fn backdrop(&mut self, rect: Rect) {
        let c = with_alpha(mix(self.theme.background, [0.0, 0.0, 0.0, 1.0], 0.5), 0.72);
        self.scene.fill(rect, c);
    }
}
