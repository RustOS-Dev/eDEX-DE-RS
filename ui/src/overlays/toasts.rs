//! Toast notifications and the volume/brightness OSD, drawn on the toast surface.

use std::time::Duration;

use crate::{
    geometry::{with_alpha, Rect},
    hit::HitTarget,
    scene::Align,
    state::ShellState,
    widgets::Ctx,
};

pub const TOAST_W: f32 = 380.0;
pub const OSD_DURATION: Duration = Duration::from_millis(1600);

/// Height needed for the toast surface given the current toasts and OSD.
pub fn required_height(state: &ShellState, line: f32) -> f32 {
    let mut h = 0.0;
    for toast in &state.toasts {
        h += toast_height(toast, line) + 8.0;
    }
    if state.osd.is_some() {
        h += line * 3.0 + 8.0;
    }
    h + 8.0
}

fn toast_height(toast: &crate::state::ToastView, line: f32) -> f32 {
    let body_lines = if toast.body.is_empty() { 0.0 } else { (toast.body.chars().count() as f32 / 44.0).ceil().min(4.0) };
    let actions = if toast.actions.is_empty() { 0.0 } else { line * 1.6 + 6.0 };
    line * 2.2 + body_lines * line + actions + 12.0
}

pub fn draw(ctx: &mut Ctx, area: Rect, state: &ShellState) {
    let t = ctx.theme;
    let line = ctx.line();
    let mut y = area.y + 8.0;
    let x = area.right() - TOAST_W - 8.0;

    if let Some(osd) = &state.osd {
        let r = Rect::new(x, y, TOAST_W, line * 3.0);
        ctx.scene.panel(r, with_alpha(t.panel_bg, 0.95), t.border, 1.0, 0.5);
        let label = if osd.muted { format!("{}  MUTED", osd.label) } else { format!("{}  {:.0}%", osd.label, osd.value * 100.0) };
        ctx.scene.text_bold(Rect::new(r.x + 12.0, r.y + 8.0, r.w - 24.0, line), ctx.metrics.ui_font, t.text_primary, Align::Left, label);
        let bar = Rect::new(r.x + 12.0, r.y + line + 14.0, r.w - 24.0, 8.0);
        ctx.scene.bar(bar, osd.value, with_alpha(t.text_secondary, 0.2), if osd.muted { t.text_dim } else { t.border });
        y += r.h + 8.0;
    }

    for toast in &state.toasts {
        let h = toast_height(toast, line);
        let r = Rect::new(x, y, TOAST_W, h);
        let accent = match toast.urgency {
            2 => t.error,
            0 => t.text_secondary,
            _ => t.border,
        };
        ctx.scene.panel(r, with_alpha(t.panel_bg, 0.96), accent, 1.0, 0.4);
        ctx.scene.fill(Rect::new(r.x, r.y, 3.0, r.h), accent);
        ctx.hits.push(r, HitTarget::Toast(toast.id));
        let s = ctx.small();
        ctx.scene.text_bold(Rect::new(r.x + 12.0, r.y + 6.0, r.w - 40.0, line), s, accent, Align::Left, toast.app.to_ascii_uppercase());
        let close = Rect::new(r.right() - 26.0, r.y + 4.0, 22.0, line);
        ctx.scene.text_aligned(close, ctx.metrics.ui_font, t.text_dim, Align::Center, "×");
        ctx.hits.push(close, HitTarget::Toast(toast.id));
        ctx.scene.text_bold(Rect::new(r.x + 12.0, r.y + 6.0 + line, r.w - 24.0, line), ctx.metrics.ui_font, t.text_primary, Align::Left, toast.summary.clone());
        let mut by = r.y + 6.0 + line * 2.2;
        if !toast.body.is_empty() {
            let body_h = h - (by - r.y) - 8.0;
            ctx.scene.paragraph(Rect::new(r.x + 12.0, by, r.w - 24.0, body_h), s, t.text_secondary, toast.body.clone());
            by += (toast.body.chars().count() as f32 / 44.0).ceil().min(4.0) * line;
        }
        if !toast.actions.is_empty() {
            let mut ax = r.x + 12.0;
            for (i, (_key, label)) in toast.actions.iter().enumerate().take(3) {
                let w = (label.chars().count() as f32 * ctx.metrics.cell_w * 0.9).round() + 20.0;
                let ar = Rect::new(ax, by + 4.0, w, line * 1.4);
                ctx.button(ar, label, HitTarget::ToastAction(toast.id * 8 + i as u32), false, true);
                ax += w + 8.0;
            }
        }
        if toast.progress > 0.0 {
            ctx.scene.fill(Rect::new(r.x + 3.0, r.bottom() - 2.0, (r.w - 3.0) * toast.progress.clamp(0.0, 1.0), 2.0), with_alpha(accent, 0.6));
        }
        y += h + 8.0;
    }
}
