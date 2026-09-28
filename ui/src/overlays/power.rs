//! Power menu overlay.

use crate::{
    geometry::{with_alpha, Rect},
    hit::HitTarget,
    scene::Align,
    state::{PowerAction, ShellState},
    widgets::Ctx,
};

pub fn draw(ctx: &mut Ctx, screen: Rect, state: &ShellState) {
    let t = ctx.theme;
    let line = ctx.line();
    let p = &state.power;
    let n = p.available.len().max(1) as f32;
    let tile = 132.0;
    let gap = 14.0;
    let w = n * tile + (n - 1.0) * gap + 40.0;
    let h = tile + line * 3.5 + 30.0;
    let panel = screen.centered(w, h).round();
    ctx.backdrop(screen);
    let inner = ctx.frame(panel, Some("POWER"));
    ctx.hits.push(panel, HitTarget::OverlayPanel);
    let mut x = inner.x + 14.0;
    for (i, action) in p.available.iter().enumerate() {
        let r = Rect::new(x, inner.y + 8.0, tile, tile);
        let selected = i == p.selected;
        let danger = matches!(action, PowerAction::Reboot | PowerAction::PowerOff);
        let color = if danger { t.error } else { t.border };
        let fill = if selected { with_alpha(color, 0.25) } else { with_alpha(color, 0.06) };
        ctx.scene.panel(r, fill, if selected { color } else { with_alpha(color, 0.5) }, 1.0, if selected { 0.9 } else { 0.0 });
        let big = (ctx.font() * 2.4).round();
        ctx.scene.text_aligned(Rect::new(r.x, r.y + 18.0, r.w, big * 1.3), big, color, Align::Center, action.glyph());
        let s = ctx.small();
        ctx.scene.text_bold(Rect::new(r.x, r.bottom() - line - 10.0, r.w, line), s, if selected { t.text_primary } else { t.text_secondary }, Align::Center, action.label());
        ctx.hits.push(r, HitTarget::OverlayItem(i as u32));
        x += tile + gap;
    }
    let hint_y = inner.y + tile + 20.0;
    let hint = match p.confirm {
        Some(a) => format!("Confirm {}? Press Enter again or click. Esc cancels.", a.label()),
        None => "← → select   Enter confirm   Esc close".to_string(),
    };
    let color = if p.confirm.is_some() { t.warning } else { t.text_dim };
    ctx.scene.text_aligned(Rect::new(inner.x, hint_y, inner.w, line), ctx.metrics.ui_font, color, Align::Center, hint);
}
