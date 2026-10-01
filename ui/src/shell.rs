//! Assembles scenes for the canvas, overlay and toast surfaces.

use std::time::Instant;

use crate::{
    geometry::{with_alpha, Rect},
    hit::{HitMap, HitTarget, ResizeHandle},
    layout::PanelLayout,
    overlays, panels,
    scene::{Align, Scanlines, Scene},
    state::{OverlayKind, ShellState},
    widgets::Ctx,
};

pub struct Rendered {
    pub scene: Scene,
    pub hits: HitMap,
}

/// Build the background canvas for one output.
pub fn render_canvas(state: &ShellState, width: f32, height: f32) -> (Rendered, PanelLayout) {
    let layout = PanelLayout::compute(width, height, &state.metrics, &state.layout_cfg);
    let mut scene = Scene::new(width, height, state.theme.background);
    let mut hits = HitMap::default();
    let pulse = state.pulse();
    {
        let mut ctx = Ctx {
            scene: &mut scene,
            hits: &mut hits,
            theme: &state.theme,
            metrics: &state.metrics,
            pulse,
        };
        panels::statusbar::draw(&mut ctx, layout.status_bar, state);
        panels::topbar::draw(&mut ctx, layout.top_bar, state);
        // With the side panels given to applications they are entirely under windows; skip them
        // (text is composited above all fills, so it would show through the strip).
        if !state.wide_tab_strip {
            panels::filesystem_view::draw(&mut ctx, layout.filesystem, state);
        }
        if !state.wide_tab_strip {
            panels::terminal_view::draw(&mut ctx, layout.terminal, state);
            panels::sysinfo::draw(&mut ctx, layout.sysinfo, state);
        }
        panels::keyboard_view::draw(&mut ctx, layout.keyboard, layout.key_h, state);
        // The tab strip itself is drawn on its own Top-layer surface (`render_strip`).
        if !state.wide_tab_strip {
            draw_handles(&mut ctx, &layout, state);
        }
        if !state.boot.done {
            draw_boot(&mut ctx, Rect::new(0.0, 0.0, width, height), state);
        }
    }
    if state.scanlines {
        scene.scanlines = Some(Scanlines {
            color: state.theme.border,
            intensity: 0.18,
        });
    }
    (Rendered { scene, hits }, layout)
}

fn draw_handles(ctx: &mut Ctx, layout: &PanelLayout, state: &ShellState) {
    let t = ctx.theme;
    for (rect, handle) in [
        (layout.fs_handle, ResizeHandle::FsTerminal),
        (layout.sysinfo_handle, ResizeHandle::TerminalSysinfo),
    ] {
        let active = state.resize.dragging == Some(handle) || state.resize.hover == Some(handle);
        let grip_h = 56.0f32.min(rect.h);
        let grip = Rect::new(
            rect.x + 1.0,
            rect.y + (rect.h - grip_h) / 2.0,
            rect.w - 2.0,
            grip_h,
        );
        ctx.scene
            .fill(grip, with_alpha(t.border, if active { 1.0 } else { 0.45 }));
        ctx.hits.push(rect, HitTarget::ResizeHandle(handle));
    }
}

fn draw_boot(ctx: &mut Ctx, screen: Rect, state: &ShellState) {
    let t = ctx.theme;
    let alpha = state.boot.overlay_alpha(state.now);
    if alpha <= 0.0 {
        return;
    }
    ctx.scene.fill(screen, with_alpha(t.background, alpha));
    let lines = state.boot.lines();
    let line_h = ctx.metrics.line * 1.2;
    let x = (screen.w * 0.12).round();
    let mut y = (screen.h * 0.3).round();
    for (i, l) in lines.iter().enumerate() {
        let color = if i == 0 || l.starts_with("SYSTEM") {
            with_alpha(t.border, alpha)
        } else {
            with_alpha(t.text_secondary, alpha)
        };
        let size = if i == 0 {
            (ctx.metrics.ui_font * 1.5).round()
        } else {
            ctx.metrics.ui_font
        };
        ctx.scene.text_bold(
            Rect::new(x, y, screen.w - x * 2.0, line_h * 1.5),
            size,
            color,
            Align::Left,
            l.clone(),
        );
        y += if i == 0 { line_h * 2.0 } else { line_h };
    }
    let s = ctx.small();
    ctx.scene.text_aligned(
        Rect::new(0.0, screen.bottom() - line_h * 2.0, screen.w, line_h),
        s,
        with_alpha(t.text_dim, alpha),
        Align::Center,
        format!("eDEX-DE {}  //  press any key to skip", state.version),
    );
}

/// Build the tab strip surface (`width` × `height` logical pixels, placed over the strip).
pub fn render_strip(state: &ShellState, width: f32, height: f32) -> Rendered {
    let mut scene = Scene::new(width, height, state.theme.panel_bg);
    let mut hits = HitMap::default();
    {
        let mut ctx = Ctx {
            scene: &mut scene,
            hits: &mut hits,
            theme: &state.theme,
            metrics: &state.metrics,
            pulse: state.pulse(),
        };
        panels::terminal_view::draw_tab_strip(&mut ctx, Rect::new(0.0, 0.0, width, height), state);
    }
    Rendered { scene, hits }
}

/// Build the overlay surface contents, if an overlay is open.
pub fn render_overlay(state: &ShellState, width: f32, height: f32) -> Option<Rendered> {
    let kind = state.overlay?;
    let mut scene = Scene::new(width, height, [0.0, 0.0, 0.0, 0.0]);
    let mut hits = HitMap::default();
    let screen = Rect::new(0.0, 0.0, width, height);
    {
        let mut ctx = Ctx {
            scene: &mut scene,
            hits: &mut hits,
            theme: &state.theme,
            metrics: &state.metrics,
            pulse: state.pulse(),
        };
        match kind {
            OverlayKind::Launcher => overlays::launcher::draw(&mut ctx, screen, state),
            OverlayKind::Power => overlays::power::draw(&mut ctx, screen, state),
            OverlayKind::Notifications => overlays::notifications::draw(&mut ctx, screen, state),
            OverlayKind::Settings => {
                overlays::form_view::draw(&mut ctx, screen, state, &state.settings)
            }
            OverlayKind::Privacy => {
                overlays::form_view::draw(&mut ctx, screen, state, &state.privacy)
            }
        }
    }
    Some(Rendered { scene, hits })
}

/// Build the toast surface contents; `None` when there is nothing to show.
pub fn render_toasts(state: &ShellState, width: f32, height: f32) -> Option<Rendered> {
    if state.toasts.is_empty() && state.osd.is_none() {
        return None;
    }
    let mut scene = Scene::new(width, height, [0.0, 0.0, 0.0, 0.0]);
    let mut hits = HitMap::default();
    {
        let mut ctx = Ctx {
            scene: &mut scene,
            hits: &mut hits,
            theme: &state.theme,
            metrics: &state.metrics,
            pulse: state.pulse(),
        };
        overlays::toasts::draw(&mut ctx, Rect::new(0.0, 0.0, width, height), state);
    }
    Some(Rendered { scene, hits })
}

/// Whether an OSD has expired.
pub fn osd_expired(state: &ShellState, now: Instant) -> bool {
    state
        .osd
        .as_ref()
        .is_some_and(|o| now.duration_since(o.shown_at) >= overlays::toasts::OSD_DURATION)
}
