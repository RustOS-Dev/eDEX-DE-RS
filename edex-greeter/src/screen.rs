//! Drawing of the greeter screen.

use ui::{
    geometry::{with_alpha, Rect},
    hit::{HitMap, HitTarget},
    layout::Metrics,
    scene::{Align, RectKind, Scene},
    theme::Theme,
    widgets::Ctx,
};

use crate::{Greeter, Phase};

pub const HIT_USER: u32 = 100;
pub const HIT_SESSION_PREV: u32 = 1;
pub const HIT_SESSION_NEXT: u32 = 2;
pub const HIT_INPUT: u32 = 3;
pub const HIT_SUBMIT: u32 = 4;
pub const HIT_REBOOT: u32 = 11;
pub const HIT_POWEROFF: u32 = 12;

pub struct Rendered {
    pub scene: Scene,
    pub hits: HitMap,
}

fn hex_grid(scene: &mut Scene, theme: &Theme, w: f32, h: f32, pulse: f32) {
    let size = 46.0;
    let dx = size * 1.5;
    let dy = size * 0.866;
    let cols = (w / dx) as i32 + 2;
    let rows = (h / dy) as i32 + 2;
    for r in 0..rows {
        for c in 0..cols {
            let x = c as f32 * dx - size + if r % 2 == 0 { 0.0 } else { dx / 2.0 };
            let y = r as f32 * dy - size;
            let d = ((x - w * 0.5).powi(2) + (y - h * 0.5).powi(2)).sqrt() / (w.max(h) * 0.7);
            let a = (0.16 - d * 0.14).max(0.02) * (0.7 + 0.3 * pulse);
            scene.shape(
                RectKind::Hexagon,
                Rect::new(x, y, size, size),
                [0.0, 0.0, 0.0, 0.0],
                with_alpha(theme.border, a),
                1.0,
            );
        }
    }
}

pub fn render(g: &Greeter, width: f32, height: f32) -> Rendered {
    let theme = &g.theme;
    let metrics: &Metrics = &g.metrics;
    let mut scene = Scene::new(width, height, theme.background);
    let mut hits = HitMap::default();
    hex_grid(&mut scene, theme, width, height, g.pulse());
    let mut ctx = Ctx {
        scene: &mut scene,
        hits: &mut hits,
        theme,
        metrics,
        pulse: g.pulse(),
    };
    let line = ctx.line();
    let font = ctx.font();

    // Header: hostname + clock.
    ctx.scene.text_bold(
        Rect::new(24.0, 16.0, width * 0.5, line * 1.4),
        font * 1.3,
        theme.border,
        Align::Left,
        format!("eDEX-OS  //  {}", g.hostname),
    );
    ctx.scene.text_aligned(
        Rect::new(width * 0.5, 16.0, width * 0.5 - 24.0, line * 1.4),
        font * 1.3,
        theme.text_primary,
        Align::Right,
        g.clock.clone(),
    );
    ctx.scene.text_aligned(
        Rect::new(width * 0.5, 16.0 + line * 1.4, width * 0.5 - 24.0, line),
        font,
        theme.text_secondary,
        Align::Right,
        g.date.clone(),
    );

    // Login panel.
    let pw = (width * 0.42).clamp(420.0, 640.0);
    let ph = line * 17.5;
    let panel = Rect::new(0.0, 0.0, width, height).centered(pw, ph).round();
    let inner = ctx.frame(panel, Some("LOGIN"));
    let mut y = inner.y + line * 0.5;

    // User list or user entry.
    if g.cfg.show_users && !g.users.is_empty() {
        ctx.label_small(Rect::new(inner.x, y, inner.w, line), "USER", theme.text_dim);
        y += line;
        let visible = 4usize;
        let start = g
            .user_idx
            .saturating_sub(visible - 1)
            .min(g.users.len().saturating_sub(visible));
        for (i, u) in g.users.iter().enumerate().skip(start).take(visible) {
            let r = Rect::new(inner.x, y, inner.w, line * 1.6);
            let selected = i == g.user_idx;
            ctx.row(r, selected, HitTarget::OverlayItem(HIT_USER + i as u32));
            ctx.scene.text_bold(
                Rect::new(r.x + 12.0, r.y + line * 0.2, r.w * 0.5, line * 1.2),
                font,
                if selected {
                    theme.text_primary
                } else {
                    theme.text_secondary
                },
                Align::Left,
                u.real_name.clone(),
            );
            ctx.scene.text_aligned(
                Rect::new(
                    r.x + r.w * 0.5,
                    r.y + line * 0.2,
                    r.w * 0.5 - 12.0,
                    line * 1.2,
                ),
                font * 0.9,
                theme.text_dim,
                Align::Right,
                u.name.clone(),
            );
            y += line * 1.6 + 2.0;
        }
        y += line * 0.4;
    } else {
        ctx.label_small(
            Rect::new(inner.x, y, inner.w, line),
            "USERNAME",
            theme.text_dim,
        );
        y += line;
        let r = Rect::new(inner.x, y, inner.w, line * 1.7);
        ctx.text_input(
            r,
            &g.username_input,
            "username",
            g.phase == Phase::PickUser,
            false,
            HitTarget::OverlayItem(HIT_INPUT),
        );
        y += line * 2.2;
    }

    // Prompt.
    let prompt_label = match g.phase {
        Phase::Prompt => g.prompt.trim().trim_end_matches(':').to_uppercase(),
        Phase::Starting => "STARTING SESSION".into(),
        Phase::Busy => "AUTHENTICATING".into(),
        Phase::PickUser => "PASSWORD".into(),
    };
    ctx.label_small(
        Rect::new(inner.x, y, inner.w, line),
        &prompt_label,
        theme.text_dim,
    );
    y += line;
    let r = Rect::new(inner.x, y, inner.w, line * 1.7);
    let editing = matches!(g.phase, Phase::Prompt)
        || (g.phase == Phase::PickUser && (g.cfg.show_users && !g.users.is_empty()));
    ctx.text_input(
        r,
        &g.input,
        if g.phase == Phase::PickUser {
            "press Enter to start"
        } else {
            ""
        },
        editing,
        g.secret,
        HitTarget::OverlayItem(HIT_INPUT),
    );
    y += line * 2.0;
    if g.caps_lock {
        ctx.scene.text_aligned(
            Rect::new(inner.x, y, inner.w, line),
            font * 0.9,
            theme.warning,
            Align::Left,
            "⚠ CAPS LOCK IS ON",
        );
    }
    y += line * 1.1;

    // Message line.
    if let Some((msg, err)) = &g.message {
        ctx.scene.paragraph(
            Rect::new(inner.x, y, inner.w, line * 2.2),
            font * 0.9,
            if *err {
                theme.error
            } else {
                theme.text_secondary
            },
            msg.clone(),
        );
    }
    y += line * 2.4;

    // Session chooser.
    ctx.label_small(
        Rect::new(inner.x, y, inner.w, line),
        "SESSION",
        theme.text_dim,
    );
    y += line;
    let name = g
        .sessions
        .get(g.session_idx)
        .map(|s| {
            if s.x11 {
                format!("{} (X11)", s.name)
            } else {
                s.name.clone()
            }
        })
        .unwrap_or_else(|| g.cfg.fallback_command.clone());
    let bw = line * 1.8;
    ctx.button(
        Rect::new(inner.x, y, bw, line * 1.5),
        "‹",
        HitTarget::OverlayItem(HIT_SESSION_PREV),
        false,
        g.sessions.len() > 1,
    );
    ctx.scene.text_aligned(
        Rect::new(inner.x + bw + 8.0, y, inner.w - 2.0 * bw - 16.0, line * 1.5),
        font,
        theme.text_primary,
        Align::Center,
        name,
    );
    ctx.button(
        Rect::new(inner.right() - bw, y, bw, line * 1.5),
        "›",
        HitTarget::OverlayItem(HIT_SESSION_NEXT),
        false,
        g.sessions.len() > 1,
    );
    y += line * 2.2;

    // Submit.
    ctx.button(
        Rect::new(inner.x, y, inner.w, line * 1.7),
        if g.lock {
            "UNLOCK"
        } else if g.phase == Phase::Prompt {
            "LOG IN"
        } else {
            "CONTINUE"
        },
        HitTarget::OverlayItem(HIT_SUBMIT),
        true,
        g.phase != Phase::Busy && g.phase != Phase::Starting,
    );

    // Power buttons.
    if g.cfg.power_buttons {
        let labels = [("REBOOT  F3", HIT_REBOOT), ("POWER OFF  F4", HIT_POWEROFF)];
        let w = 150.0;
        let mut x = width - 24.0 - w * labels.len() as f32 - 8.0;
        for (label, id) in labels {
            let r = Rect::new(x, height - 24.0 - line * 1.6, w, line * 1.6);
            ctx.button(r, label, HitTarget::OverlayItem(id), false, true);
            x += w + 8.0;
        }
    }
    ctx.label_small(
        Rect::new(24.0, height - 24.0 - line * 1.6, width * 0.5, line * 1.6),
        &if g.lock {
            format!(
                "edex-greeter {}  ·  locked  ·  Enter unlock",
                env!("CARGO_PKG_VERSION")
            )
        } else {
            format!(
                "edex-greeter {}  ·  ↑↓ user  ·  Tab session  ·  Enter log in",
                env!("CARGO_PKG_VERSION")
            )
        },
        theme.text_dim,
    );

    Rendered { scene, hits }
}
