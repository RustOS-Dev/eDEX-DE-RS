//! Notification history overlay.

use crate::{
    geometry::{with_alpha, Rect},
    hit::HitTarget,
    scene::Align,
    state::ShellState,
    widgets::Ctx,
};

pub const HIT_CLEAR_ALL: u32 = 0xffff_0001;
pub const HIT_TOGGLE_DND: u32 = 0xffff_0002;

pub fn draw(ctx: &mut Ctx, screen: Rect, state: &ShellState) {
    let t = ctx.theme;
    let line = ctx.line();
    let w = (screen.w * 0.42).clamp(380.0, 640.0);
    let h = (screen.h * 0.7).round();
    let panel = Rect::new(screen.right() - w - 16.0, (screen.h - h) / 2.0, w, h).round();
    ctx.backdrop(screen);
    let inner = ctx.frame(panel, Some("NOTIFICATIONS"));
    ctx.hits.push(panel, HitTarget::OverlayPanel);
    let n = &state.notifications;

    let btn_h = line * 1.5;
    let dnd_label = if n.dnd { "DO NOT DISTURB: ON" } else { "DO NOT DISTURB: OFF" };
    ctx.button(Rect::new(inner.x, inner.y, inner.w * 0.55 - 4.0, btn_h), dnd_label, HitTarget::OverlayItem(HIT_TOGGLE_DND), false, true);
    ctx.button(Rect::new(inner.x + inner.w * 0.55 + 4.0, inner.y, inner.w * 0.45 - 4.0, btn_h), "CLEAR ALL", HitTarget::OverlayItem(HIT_CLEAR_ALL), false, !n.rows.is_empty());

    let list = Rect::new(inner.x, inner.y + btn_h + 10.0, inner.w, inner.h - btn_h - 10.0);
    if n.rows.is_empty() {
        ctx.label(Rect::new(list.x + 6.0, list.y, list.w, line), "no notifications", t.text_dim);
        return;
    }
    let row_h = line * 3.2;
    let visible = (list.h / row_h).floor().max(1.0) as usize;
    let start = n.selected.saturating_sub(visible - 1).min(n.rows.len().saturating_sub(visible));
    for (vi, (i, row)) in n.rows.iter().enumerate().skip(start).take(visible).enumerate() {
        let r = Rect::new(list.x, list.y + vi as f32 * row_h, list.w, row_h - 4.0);
        let urgent = row.urgency >= 2;
        ctx.scene.fill(r, with_alpha(if urgent { t.error } else { t.border }, if i == n.selected { 0.18 } else { 0.06 }));
        ctx.scene.fill(Rect::new(r.x, r.y, 2.0, r.h), if urgent { t.error } else { t.border });
        ctx.hits.push(r, HitTarget::OverlayItem(row.id));
        let s = ctx.small();
        ctx.scene.text_bold(Rect::new(r.x + 10.0, r.y + 4.0, r.w * 0.6, line), s, t.border, Align::Left, row.app.to_ascii_uppercase());
        ctx.scene.text_aligned(Rect::new(r.x, r.y + 4.0, r.w - 10.0, line), s, t.text_dim, Align::Right, row.time.clone());
        ctx.label(Rect::new(r.x + 10.0, r.y + 4.0 + line, r.w - 20.0, line), &row.summary, t.text_primary);
        ctx.label_small(Rect::new(r.x + 10.0, r.y + 4.0 + line * 2.0, r.w - 20.0, line), &row.body, t.text_secondary);
    }
}
