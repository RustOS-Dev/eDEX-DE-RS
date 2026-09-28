//! Top bar: logo, hostname, workspaces, active window, keyboard layout, clock.

use crate::{
    geometry::{with_alpha, Rect},
    hit::HitTarget,
    scene::Align,
    state::ShellState,
    widgets::Ctx,
};

pub fn draw(ctx: &mut Ctx, rect: Rect, state: &ShellState) {
    let t = ctx.theme;
    ctx.scene.fill(rect, t.panel_bg);
    ctx.scene.hline(
        rect.x,
        rect.bottom() - 1.0,
        rect.w,
        with_alpha(t.border, 0.6),
    );
    let line = ctx.line();
    let ty = rect.y + (rect.h - line) / 2.0;
    let mut x = rect.x + 12.0;

    // Logo block
    let logo_w = 96.0;
    let logo = Rect::new(x, rect.y + 4.0, logo_w, rect.h - 8.0);
    ctx.scene.fill(logo, with_alpha(t.border, 0.15));
    ctx.scene.stroke(logo, t.border, 1.0);
    let size = ctx.font();
    ctx.scene.text_bold(
        Rect::new(logo.x, ty, logo.w, line),
        size,
        t.border,
        Align::Center,
        "eDEX-DE",
    );
    ctx.hits.push(logo, HitTarget::Launcher);
    x += logo_w + 14.0;

    // Host
    let host = format!("{}@{}", state.username, state.hostname);
    let host_w = (host.chars().count() as f32 * ctx.metrics.cell_w * 0.95).round() + 8.0;
    ctx.label(Rect::new(x, ty, host_w, line), &host, t.text_secondary);
    x += host_w + 16.0;

    // Workspaces
    if state.hypr_connected && !state.workspaces.is_empty() {
        let ws_h = rect.h - 10.0;
        for ws in &state.workspaces {
            let w = if ws.name.chars().count() > 2 {
                40.0
            } else {
                ws_h
            };
            let r = Rect::new(x, rect.y + 5.0, w, ws_h);
            let (fill, fg) = if ws.active {
                (t.border, t.background)
            } else if ws.windows > 0 {
                (with_alpha(t.border, 0.25), t.text_primary)
            } else {
                (with_alpha(t.border, 0.08), t.text_dim)
            };
            ctx.scene.fill(r, fill);
            let s = ctx.small();
            ctx.scene.text_aligned(
                Rect::new(r.x, r.y + (r.h - line) / 2.0, r.w, line),
                s,
                fg,
                Align::Center,
                ws.name.clone(),
            );
            ctx.hits.push(r, HitTarget::Workspace(ws.id.max(0) as u32));
            x += w + 4.0;
        }
        x += 12.0;
    }

    // Live ISO install button
    let mut right = rect.right() - 12.0;
    if state.live_iso {
        let w = 150.0;
        let r = Rect::new(right - w, rect.y + 5.0, w, rect.h - 10.0);
        ctx.scene
            .panel(r, with_alpha(t.accent, 0.2), t.accent, 1.0, 0.8 * ctx.pulse);
        let s = ctx.small();
        ctx.scene.text_bold(
            Rect::new(r.x, r.y + (r.h - line) / 2.0, r.w, line),
            s,
            t.accent,
            Align::Center,
            "▶ INSTALL eDEX-OS",
        );
        ctx.hits.push(r, HitTarget::Install);
        right -= w + 12.0;
    }

    // Clock / date
    let clock = format!("{}  {}", state.date, state.clock);
    let clock_w = (clock.chars().count() as f32 * ctx.metrics.cell_w).round() + 12.0;
    let cr = Rect::new(right - clock_w, ty, clock_w, line);
    ctx.scene
        .text_bold(cr, ctx.metrics.ui_font, t.text_primary, Align::Right, clock);
    ctx.hits.push(cr, HitTarget::Clock);
    right -= clock_w + 16.0;

    // Keyboard layout
    let kb = state.kb_layout.to_ascii_uppercase();
    let kb_w = (kb.chars().count() as f32 * ctx.metrics.cell_w).round() + 8.0;
    ctx.label_right(
        Rect::new(right - kb_w, ty, kb_w, line),
        &kb,
        t.text_secondary,
    );
    right -= kb_w + 16.0;

    // Active window title (center, remaining space)
    if right > x + 40.0 {
        let title = state
            .active_window
            .clone()
            .unwrap_or_else(|| "// no active window".to_string());
        ctx.scene.text_aligned(
            Rect::new(x, ty, right - x, line),
            ctx.metrics.ui_font,
            t.text_secondary,
            Align::Center,
            title,
        );
    }
}
