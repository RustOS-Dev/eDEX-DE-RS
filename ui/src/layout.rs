//! Panel layout in logical pixels, derived from font metrics and the user's split ratios.

use crate::geometry::Rect;

/// Font metrics measured by the renderer for the current font size (logical pixels).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Metrics {
    /// UI font size.
    pub ui_font: f32,
    /// UI line height.
    pub line: f32,
    /// Terminal cell width.
    pub cell_w: f32,
    /// Terminal cell height.
    pub cell_h: f32,
    /// Terminal font size.
    pub term_font: f32,
}

impl Default for Metrics {
    fn default() -> Self {
        Self {
            ui_font: 14.0,
            line: 20.0,
            cell_w: 8.4,
            cell_h: 18.0,
            term_font: 13.0,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct LayoutConfig {
    /// Fraction of the width taken by the filesystem panel.
    pub fs_split: f32,
    /// Fraction of the width where the sysinfo panel starts.
    pub sysinfo_split: f32,
    pub keyboard_visible: bool,
}

impl Default for LayoutConfig {
    fn default() -> Self {
        Self {
            fs_split: 0.20,
            sysinfo_split: 0.78,
            keyboard_visible: true,
        }
    }
}

pub const KEYBOARD_ROWS: usize = 5;
pub const KEY_GAP: f32 = 6.0;
pub const KEYBOARD_PAD: f32 = 10.0;

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct PanelLayout {
    pub width: f32,
    pub height: f32,
    pub status_bar: Rect,
    pub top_bar: Rect,
    pub filesystem: Rect,
    pub terminal: Rect,
    pub sysinfo: Rect,
    pub keyboard: Rect,
    pub fs_handle: Rect,
    pub sysinfo_handle: Rect,
    /// Height of one keyboard key.
    pub key_h: f32,
}

impl PanelLayout {
    pub fn compute(width: f32, height: f32, metrics: &Metrics, cfg: &LayoutConfig) -> Self {
        let status_h = (metrics.line * 1.4).round();
        let top_h = (metrics.line * 2.0).round();
        let key_h = (metrics.line * 1.8).round().max(28.0);
        let keyboard_h = if cfg.keyboard_visible {
            (KEYBOARD_ROWS as f32 * key_h
                + (KEYBOARD_ROWS as f32 - 1.0) * KEY_GAP
                + KEYBOARD_PAD * 2.0)
                .round()
        } else {
            0.0
        };
        let panel_y = status_h + top_h;
        let panel_h = (height - panel_y - keyboard_h).max(0.0);

        let min_fs = (metrics.cell_w * 16.0).round();
        let min_term = (metrics.cell_w * 40.0).round();
        let min_sys = (metrics.cell_w * 24.0).round();
        let max_fs = (width - min_term - min_sys).max(min_fs);
        let fs_w = (width * cfg.fs_split).round().clamp(min_fs, max_fs);
        let min_sys_x = fs_w + min_term;
        let max_sys_x = (width - min_sys).max(min_sys_x);
        let sys_x = (width * cfg.sysinfo_split)
            .round()
            .clamp(min_sys_x, max_sys_x);

        let handle_w = 6.0;
        Self {
            width,
            height,
            status_bar: Rect::new(0.0, 0.0, width, status_h),
            top_bar: Rect::new(0.0, status_h, width, top_h),
            filesystem: Rect::new(0.0, panel_y, fs_w, panel_h),
            terminal: Rect::new(fs_w, panel_y, (sys_x - fs_w).max(0.0), panel_h),
            sysinfo: Rect::new(sys_x, panel_y, (width - sys_x).max(0.0), panel_h),
            keyboard: Rect::new(0.0, height - keyboard_h, width, keyboard_h),
            fs_handle: Rect::new(fs_w - handle_w / 2.0, panel_y, handle_w, panel_h),
            sysinfo_handle: Rect::new(sys_x - handle_w / 2.0, panel_y, handle_w, panel_h),
            key_h,
        }
    }

    /// Exclusive zones (top, bottom, left, right) the reservers should claim.
    pub fn reserved_zones(&self) -> (u32, u32, u32, u32) {
        (
            (self.status_bar.h + self.top_bar.h).round() as u32,
            self.keyboard.h.round() as u32,
            self.filesystem.w.round() as u32,
            self.sysinfo.w.round() as u32,
        )
    }

    /// Terminal grid dimensions that fit the terminal panel's content area.
    pub fn terminal_grid(&self, metrics: &Metrics, tab_bar_h: f32) -> (usize, usize) {
        let inner = self.terminal.inset(8.0).below(tab_bar_h);
        let cols = (inner.w / metrics.cell_w).floor().max(20.0) as usize;
        let rows = (inner.h / metrics.cell_h).floor().max(5.0) as usize;
        (cols, rows)
    }

    /// Convert a pointer x position into an fs split ratio.
    pub fn split_from_x(&self, x: f32) -> f32 {
        (x / self.width.max(1.0)).clamp(0.05, 0.95)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn layout_snapshots() {
        let m = Metrics::default();
        let cfg = LayoutConfig::default();
        for (w, h) in [
            (1280.0, 720.0),
            (1920.0, 1080.0),
            (2560.0, 1440.0),
            (3840.0, 2160.0),
        ] {
            let l = PanelLayout::compute(w, h, &m, &cfg);
            insta::assert_debug_snapshot!(format!("layout_{}x{}", w as u32, h as u32), l);
        }
    }

    #[test]
    fn panels_tile_the_full_area() {
        let l = PanelLayout::compute(
            1920.0,
            1080.0,
            &Metrics::default(),
            &LayoutConfig::default(),
        );
        assert_eq!(l.filesystem.w + l.terminal.w + l.sysinfo.w, 1920.0);
        assert_eq!(l.filesystem.y, l.status_bar.h + l.top_bar.h);
        assert_eq!(l.filesystem.bottom(), l.keyboard.y);
        let (top, bottom, left, right) = l.reserved_zones();
        assert_eq!(top as f32, l.status_bar.h + l.top_bar.h);
        assert_eq!(bottom as f32, l.keyboard.h);
        assert_eq!(left as f32, l.filesystem.w);
        assert_eq!(right as f32, l.sysinfo.w);
    }

    #[test]
    fn hidden_keyboard_frees_space() {
        let cfg = LayoutConfig {
            keyboard_visible: false,
            ..Default::default()
        };
        let l = PanelLayout::compute(1920.0, 1080.0, &Metrics::default(), &cfg);
        assert_eq!(l.keyboard.h, 0.0);
        assert_eq!(l.terminal.bottom(), 1080.0);
    }

    #[test]
    fn narrow_screens_respect_minimums() {
        let l = PanelLayout::compute(800.0, 600.0, &Metrics::default(), &LayoutConfig::default());
        assert!(l.terminal.w >= 8.4 * 40.0 - 1.0);
        assert!(l.filesystem.w >= 8.4 * 16.0 - 1.0);
    }
}
