//! Renders a `TabbedForms` overlay (settings, privacy panel).

use crate::{
    form::{hit_id, ControlKind, PART_ACTION, PART_DEC, PART_INC, PART_MAIN, PART_SLIDER},
    geometry::{with_alpha, Rect},
    hit::HitTarget,
    scene::Align,
    state::{ShellState, TabbedForms},
    widgets::Ctx,
};

pub const HIT_TAB_BASE: u32 = 0xfff0_0000;
pub const HIT_CLOSE: u32 = 0xffff_fff0;

pub fn draw(ctx: &mut Ctx, screen: Rect, _state: &ShellState, forms: &TabbedForms) {
    let t = ctx.theme;
    let line = ctx.line();
    let w = (screen.w * 0.72).clamp(640.0, 1200.0);
    let h = (screen.h * 0.8).clamp(420.0, 900.0).min(screen.h - 20.0);
    let panel = screen.centered(w, h).round();
    ctx.backdrop(screen);
    let inner = ctx.frame(panel, Some(&forms.title));
    ctx.hits.push(panel, HitTarget::OverlayPanel);
    let close = Rect::new(panel.right() - 30.0, panel.y + 2.0, 26.0, line);
    ctx.scene.text_aligned(
        close,
        ctx.metrics.ui_font,
        t.text_secondary,
        Align::Center,
        "×",
    );
    ctx.hits.push(close, HitTarget::OverlayItem(HIT_CLOSE));

    // Sidebar with tabs
    let side_w = (inner.w * 0.24).clamp(150.0, 220.0);
    let side = Rect::new(inner.x, inner.y, side_w, inner.h);
    let tab_h = line * 1.7;
    for (i, name) in forms.tabs.iter().enumerate() {
        let r = Rect::new(side.x, side.y + i as f32 * tab_h, side.w - 8.0, tab_h - 2.0);
        let active = i == forms.active;
        if active {
            ctx.scene.fill(r, with_alpha(t.border, 0.22));
            ctx.scene.fill(Rect::new(r.x, r.y, 3.0, r.h), t.border);
        }
        let s = ctx.small();
        ctx.scene.text_bold(
            Rect::new(r.x + 12.0, r.y + (r.h - line) / 2.0, r.w - 12.0, line),
            s,
            if active {
                t.text_primary
            } else {
                t.text_secondary
            },
            Align::Left,
            name.to_ascii_uppercase(),
        );
        ctx.hits
            .push(r, HitTarget::OverlayItem(HIT_TAB_BASE + i as u32));
    }
    ctx.scene.vline(
        side.right() - 4.0,
        side.y,
        side.h,
        with_alpha(t.border, 0.3),
    );

    // Content
    let content = Rect::new(
        side.right() + 8.0,
        inner.y,
        inner.w - side_w - 8.0,
        inner.h
            - if forms.status.is_some() {
                line * 1.4
            } else {
                0.0
            },
    );
    draw_form(ctx, content, forms);
    if let Some(status) = &forms.status {
        let r = Rect::new(
            content.x,
            inner.bottom() - line * 1.2,
            content.w,
            line * 1.2,
        );
        ctx.scene.fill(r, with_alpha(t.border, 0.1));
        ctx.label_small(
            Rect::new(r.x + 8.0, r.y + (r.h - line) / 2.0, r.w - 16.0, line),
            status,
            t.text_secondary,
        );
    }
}

fn draw_form(ctx: &mut Ctx, area: Rect, forms: &TabbedForms) {
    let t = ctx.theme;
    let line = ctx.line();
    let row_h = line * 1.9;
    let fs = &forms.form_state;
    let mut y = area.y - fs.scroll;
    let label_w = (area.w * 0.42).round();
    let value_x = area.x + label_w + 12.0;
    let value_w = area.w - label_w - 16.0;

    for section in &forms.form.sections {
        if !section.title.is_empty() {
            if y + line > area.y && y < area.bottom() {
                ctx.heading(Rect::new(area.x, y, area.w, line * 1.2), &section.title);
                ctx.scene
                    .hline(area.x, y + line * 1.2, area.w, with_alpha(t.border, 0.35));
            }
            y += line * 1.6;
        }
        for control in &section.controls {
            let height = control_height(control, line, row_h, value_w, ctx.metrics.cell_w);
            let r = Rect::new(area.x, y, area.w, height);
            if r.bottom() > area.y && r.y < area.bottom() {
                let focused = fs.focused == Some(control.id);
                if focused {
                    ctx.scene.fill(r, with_alpha(t.border, 0.08));
                }
                let fg = if control.enabled {
                    t.text_primary
                } else {
                    t.text_dim
                };
                let label_rect =
                    Rect::new(r.x + 6.0, r.y + (row_h - line) / 2.0, label_w - 6.0, line);
                match &control.kind {
                    ControlKind::Separator => {
                        ctx.scene
                            .hline(r.x, r.y + height / 2.0, r.w, with_alpha(t.border, 0.25));
                    }
                    ControlKind::Note(text) => {
                        let s = ctx.small();
                        ctx.scene.paragraph(
                            Rect::new(r.x + 6.0, r.y + 2.0, r.w - 12.0, height - 4.0),
                            s,
                            t.text_secondary,
                            text.clone(),
                        );
                    }
                    ControlKind::Info(value) => {
                        ctx.label(label_rect, &control.label, t.text_secondary);
                        ctx.label(Rect::new(value_x, label_rect.y, value_w, line), value, fg);
                    }
                    ControlKind::Toggle(on) => {
                        ctx.label(label_rect, &control.label, fg);
                        let tr = Rect::new(value_x, r.y + (row_h - 22.0) / 2.0, 46.0, 22.0);
                        ctx.toggle(
                            tr,
                            *on,
                            HitTarget::OverlayItem(hit_id(control.id, PART_MAIN)),
                            focused,
                            control.enabled,
                        );
                        let s = ctx.small();
                        ctx.scene.text(
                            Rect::new(tr.right() + 10.0, label_rect.y, value_w - 56.0, line),
                            s,
                            t.text_secondary,
                            if *on { "ON" } else { "OFF" },
                        );
                    }
                    ControlKind::Slider {
                        value,
                        min,
                        max,
                        unit,
                        ..
                    } => {
                        ctx.label(label_rect, &control.label, fg);
                        let frac = if max > min {
                            (value - min) / (max - min)
                        } else {
                            0.0
                        };
                        let text = format!("{:.0}{}", value, unit);
                        let text_w = 64.0;
                        let sr = Rect::new(value_x, r.y, value_w - text_w - 8.0, row_h);
                        if control.enabled {
                            ctx.slider(
                                sr,
                                frac,
                                HitTarget::OverlayItem(hit_id(control.id, PART_SLIDER)),
                                focused,
                            );
                        } else {
                            ctx.scene.fill(
                                Rect::new(sr.x, sr.y + sr.h / 2.0 - 2.0, sr.w, 4.0),
                                with_alpha(t.text_secondary, 0.15),
                            );
                        }
                        ctx.label_right(
                            Rect::new(sr.right() + 8.0, label_rect.y, text_w, line),
                            &text,
                            fg,
                        );
                    }
                    ControlKind::Choice { options, selected } => {
                        ctx.label(label_rect, &control.label, fg);
                        let mut cx = value_x;
                        let cy = r.y + (row_h - line * 1.3) / 2.0;
                        let mut wrapped = 0.0;
                        for (i, opt) in options.iter().enumerate() {
                            let w = chip_width(opt, ctx.metrics.cell_w);
                            if cx + w > value_x + value_w && cx > value_x {
                                cx = value_x;
                                wrapped += line * 1.5;
                            }
                            let cr = Rect::new(cx, cy + wrapped, w, line * 1.3);
                            if control.enabled {
                                ctx.chip(
                                    cr,
                                    opt,
                                    i == *selected,
                                    HitTarget::OverlayItem(hit_id(control.id, i as u8 + 1)),
                                );
                            } else {
                                ctx.scene.fill_rounded(cr, with_alpha(t.border, 0.06), 3.0);
                                ctx.label_center(
                                    Rect::new(cr.x, cr.y + (cr.h - line) / 2.0, cr.w, line),
                                    opt,
                                    t.text_dim,
                                );
                            }
                            cx += w + 6.0;
                        }
                    }
                    ControlKind::Button(text) => {
                        ctx.label(label_rect, &control.label, fg);
                        let w =
                            (text.chars().count() as f32 * ctx.metrics.cell_w * 0.9).round() + 28.0;
                        ctx.button(
                            Rect::new(
                                value_x,
                                r.y + (row_h - line * 1.5) / 2.0,
                                w.min(value_w),
                                line * 1.5,
                            ),
                            text,
                            HitTarget::OverlayItem(hit_id(control.id, PART_MAIN)),
                            focused,
                            control.enabled,
                        );
                    }
                    ControlKind::Text {
                        value,
                        secret,
                        placeholder,
                    } => {
                        ctx.label(label_rect, &control.label, fg);
                        let editing = fs.editing == Some(control.id);
                        ctx.text_input(
                            Rect::new(
                                value_x,
                                r.y + (row_h - line * 1.5) / 2.0,
                                value_w,
                                line * 1.5,
                            ),
                            value,
                            placeholder,
                            editing,
                            *secret,
                            HitTarget::OverlayItem(hit_id(control.id, PART_MAIN)),
                        );
                    }
                    ControlKind::Progress { frac, label } => {
                        ctx.label(label_rect, &control.label, t.text_secondary);
                        let br = Rect::new(value_x, r.y + row_h / 2.0 - 5.0, value_w - 90.0, 10.0);
                        ctx.meter(br, *frac, t.border);
                        ctx.label_right(
                            Rect::new(br.right() + 6.0, label_rect.y, 84.0, line),
                            label,
                            fg,
                        );
                    }
                    ControlKind::List {
                        items,
                        action_label,
                        empty,
                    } => {
                        ctx.label(
                            Rect::new(r.x + 6.0, r.y + 2.0, area.w, line),
                            &control.label,
                            t.text_secondary,
                        );
                        let mut ly = r.y + line + 4.0;
                        if items.is_empty() {
                            ctx.label_small(
                                Rect::new(r.x + 16.0, ly, r.w - 24.0, line),
                                empty,
                                t.text_dim,
                            );
                        }
                        let cursor = fs.list_cursor(control.id);
                        for (i, item) in items.iter().enumerate() {
                            let ir = Rect::new(r.x + 10.0, ly, r.w - 20.0, line * 1.5);
                            let highlighted = focused && cursor == i;
                            if item.selected {
                                ctx.scene.fill(ir, with_alpha(t.accent, 0.12));
                            }
                            if highlighted {
                                ctx.scene.stroke(ir, t.border, 1.0);
                            }
                            ctx.hits
                                .push(ir, HitTarget::OverlayItem(hit_id(control.id, i as u8 + 1)));
                            let mark = if item.selected { "● " } else { "○ " };
                            ctx.label(
                                Rect::new(ir.x + 8.0, ir.y + (ir.h - line) / 2.0, ir.w * 0.5, line),
                                &format!("{mark}{}", item.label),
                                if item.selected { t.accent } else { fg },
                            );
                            let s = ctx.small();
                            let detail_w =
                                ir.w * 0.5 - if action_label.is_some() { 110.0 } else { 8.0 };
                            ctx.scene.text_aligned(
                                Rect::new(
                                    ir.x + ir.w * 0.5,
                                    ir.y + (ir.h - line) / 2.0,
                                    detail_w,
                                    line,
                                ),
                                s,
                                t.text_secondary,
                                Align::Right,
                                item.detail.clone(),
                            );
                            if let Some(badge) = &item.badge {
                                let bw = (badge.chars().count() as f32 * ctx.metrics.cell_w * 0.8)
                                    .round()
                                    + 10.0;
                                let br = Rect::new(
                                    ir.x + ir.w * 0.5 - bw - 4.0,
                                    ir.y + 4.0,
                                    bw,
                                    ir.h - 8.0,
                                );
                                let _ = br;
                            }
                            if let Some(al) = action_label {
                                let ar =
                                    Rect::new(ir.right() - 100.0, ir.y + 3.0, 94.0, ir.h - 6.0);
                                ctx.button(
                                    ar,
                                    al,
                                    HitTarget::OverlayItem(
                                        hit_id(control.id, PART_ACTION) + i as u32 * 0x0100_0000,
                                    ),
                                    false,
                                    control.enabled,
                                );
                            }
                            ly += line * 1.5 + 2.0;
                        }
                    }
                }
                // Keyboard hints for focused adjustable controls
                if focused
                    && matches!(
                        control.kind,
                        ControlKind::Slider { .. } | ControlKind::Choice { .. }
                    )
                {
                    let s = ctx.small();
                    ctx.scene.text_aligned(
                        Rect::new(r.x, r.bottom() - line * 0.9, r.w - 6.0, line * 0.9),
                        s,
                        t.text_dim,
                        Align::Right,
                        "← → adjust",
                    );
                    let _ = (PART_DEC, PART_INC);
                }
            }
            y += height;
        }
        y += line * 0.5;
    }
}

fn chip_width(option: &str, cell_w: f32) -> f32 {
    (option.chars().count() as f32 * cell_w * 0.9).round() + 18.0
}

/// Rows a choice control's chips wrap onto, using the same flow as the drawing code.
fn choice_rows(options: &[String], value_w: f32, cell_w: f32) -> usize {
    let mut rows = 1;
    let mut x = 0.0;
    for opt in options {
        let w = chip_width(opt, cell_w);
        if x + w > value_w && x > 0.0 {
            rows += 1;
            x = 0.0;
        }
        x += w + 6.0;
    }
    rows
}

pub fn control_height(
    control: &crate::form::Control,
    line: f32,
    row_h: f32,
    value_w: f32,
    cell_w: f32,
) -> f32 {
    match &control.kind {
        ControlKind::Separator => line * 0.8,
        ControlKind::Note(text) => {
            (text.chars().count() as f32 / 80.0).ceil().max(1.0) * line * 1.4 + 8.0
        }
        ControlKind::List { items, .. } => {
            line + 6.0 + items.len().max(1) as f32 * (line * 1.5 + 2.0) + 6.0
        }
        ControlKind::Choice { options, .. } => {
            row_h + (choice_rows(options, value_w, cell_w) - 1) as f32 * line * 1.5
        }
        _ => row_h,
    }
}

/// Total form height, used for scroll clamping.
pub fn form_height(forms: &TabbedForms, line: f32, value_w: f32, cell_w: f32) -> f32 {
    let row_h = line * 1.9;
    let mut h = 0.0;
    for section in &forms.form.sections {
        if !section.title.is_empty() {
            h += line * 1.6;
        }
        for c in &section.controls {
            h += control_height(c, line, row_h, value_w, cell_w);
        }
        h += line * 0.5;
    }
    h
}

#[cfg(test)]
mod tests {
    use super::*;

    fn themes() -> Vec<String> {
        [
            "amber",
            "apollo",
            "blade",
            "cyborg",
            "horizon",
            "interstellar",
            "matrix",
            "navy",
            "nord",
            "purple",
            "red",
            "tron",
        ]
        .iter()
        .map(|s| s.to_string())
        .collect()
    }

    /// The reserved height must fit every wrapped row, or chips overlap the next control.
    #[test]
    fn choice_height_covers_all_wrapped_rows() {
        let (line, row_h, cell_w) = (18.0, 18.0 * 1.9, 8.4);
        for value_w in [240.0, 380.0, 520.0, 900.0] {
            let rows = choice_rows(&themes(), value_w, cell_w);
            let control = crate::form::Control::new(
                1,
                "Theme",
                ControlKind::Choice {
                    options: themes(),
                    selected: 0,
                },
            );
            let h = control_height(&control, line, row_h, value_w, cell_w);
            let last_row_bottom =
                (row_h - line * 1.3) / 2.0 + (rows - 1) as f32 * line * 1.5 + line * 1.3;
            assert!(
                h >= last_row_bottom,
                "width {value_w}: {rows} rows need {last_row_bottom}, got {h}"
            );
        }
        assert_eq!(choice_rows(&themes(), 380.0, 8.4), 3);
        assert_eq!(choice_rows(&themes(), 2000.0, 8.4), 1);
    }
}
