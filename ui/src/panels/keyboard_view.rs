//! On-screen keyboard drawing.

use crate::{
    geometry::{with_alpha, Rect},
    hit::HitTarget,
    keyboard::{key_rect, KeyAction, ROWS},
    scene::{Align, RectKind},
    state::ShellState,
    widgets::Ctx,
};

pub fn draw(ctx: &mut Ctx, rect: Rect, key_h: f32, state: &ShellState) {
    if rect.h <= 0.0 {
        return;
    }
    let t = ctx.theme;
    ctx.scene.fill(rect, t.background);
    ctx.scene
        .hline(rect.x, rect.y, rect.w, with_alpha(t.border, 0.5));
    let kb = &state.keyboard;
    let shifted = kb.shift || kb.sticky_shift || kb.caps_lock;
    let size = ctx.small();
    let line = ctx.line();
    for (r, row) in ROWS.iter().enumerate() {
        for (c, key) in row.iter().enumerate() {
            let kr = key_rect(rect, key_h, r, c);
            let pressed = kb.is_pressed(r, c) || kb.modifier_active(key.action);
            let hovered = kb.hover == Some((r, c));
            let fill = if pressed {
                t.key_active
            } else if hovered {
                with_alpha(t.border, 0.3)
            } else {
                t.key_fill
            };
            let border = if pressed { t.border } else { t.key_border };
            ctx.scene.shape(RectKind::KeyCap, kr, fill, border, 1.0);
            let label = if shifted && matches!(key.action, KeyAction::Char(_)) {
                key.shifted
            } else {
                key.label
            };
            let fg = if pressed {
                t.background
            } else if matches!(key.action, KeyAction::Char(_)) {
                t.text_primary
            } else {
                t.text_secondary
            };
            ctx.scene.text_aligned(
                Rect::new(kr.x, kr.y + (kr.h - line) / 2.0, kr.w, line),
                size,
                fg,
                Align::Center,
                label,
            );
            ctx.hits.push(kr, HitTarget::KeyboardKey(r, c));
        }
    }
}
