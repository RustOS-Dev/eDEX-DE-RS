//! Application launcher overlay.

use crate::{
    geometry::{with_alpha, Rect},
    hit::HitTarget,
    scene::Align,
    state::ShellState,
    widgets::Ctx,
};

pub const VISIBLE_RESULTS: usize = 8;

pub fn draw(ctx: &mut Ctx, screen: Rect, state: &ShellState) {
    let t = ctx.theme;
    let line = ctx.line();
    let row_h = (line * 1.9).round();
    let w = (screen.w * 0.5).clamp(420.0, 760.0);
    let h = line * 2.6 + row_h * VISIBLE_RESULTS as f32 + 24.0;
    let panel = Rect::new((screen.w - w) / 2.0, (screen.h * 0.18).round(), w, h).round();
    ctx.backdrop(screen);
    let inner = ctx.frame(panel, Some("LAUNCH  //  type to search, Enter to run, Ctrl+Enter to run in terminal"));
    ctx.hits.push(panel, HitTarget::OverlayPanel);

    let input = Rect::new(inner.x, inner.y, inner.w, line * 1.6);
    let l = &state.launcher;
    ctx.text_input(input, &l.query, "search applications…", true, false, HitTarget::OverlayPanel);

    let list_y = input.bottom() + 8.0;
    if l.results.is_empty() {
        ctx.label(Rect::new(inner.x + 6.0, list_y, inner.w, line), if l.query.is_empty() { "no applications found" } else { "no matches" }, t.text_dim);
        return;
    }
    let start = l.scroll.min(l.results.len().saturating_sub(1));
    for (vi, (i, res)) in l.results.iter().enumerate().skip(start).take(VISIBLE_RESULTS).enumerate() {
        let r = Rect::new(inner.x, list_y + vi as f32 * row_h, inner.w, row_h);
        ctx.row(r, i == l.selected, HitTarget::OverlayItem(i as u32));
        let name_color = if i == l.selected { t.text_primary } else { t.text_secondary };
        ctx.label(Rect::new(r.x + 10.0, r.y + 2.0, r.w * 0.6, line), &res.name, name_color);
        if let Some(c) = &res.comment {
            let s = ctx.small();
            ctx.scene.text(Rect::new(r.x + 10.0, r.y + line - 2.0, r.w - 20.0, line), s, t.text_dim, c.clone());
        }
        if let Some(cat) = &res.category {
            let s = ctx.small();
            ctx.scene.text_aligned(Rect::new(r.x, r.y + 2.0, r.w - 10.0, line), s, with_alpha(t.border, 0.7), Align::Right, cat.to_ascii_uppercase());
        }
    }
    if l.results.len() > VISIBLE_RESULTS {
        let s = ctx.small();
        ctx.scene.text_aligned(Rect::new(inner.x, inner.bottom() - line, inner.w, line), s, t.text_dim, Align::Right, format!("{} / {}", l.selected + 1, l.results.len()));
    }
}
