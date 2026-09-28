//! Terminal panel: tab bar and cell grid.

use crate::{
    geometry::{with_alpha, Rect},
    hit::HitTarget,
    scene::{Align, TextSpan},
    state::{PanelFocus, ShellState},
    terminal_model::CursorShape,
    widgets::Ctx,
};

/// Height of the tab bar for the given metrics.
pub fn tab_bar_height(line: f32) -> f32 {
    (line * 1.3).round()
}

/// The rectangle the grid is drawn into (used to size the PTY).
pub fn grid_rect(panel: Rect, line: f32) -> Rect {
    panel.inset(8.0).below(tab_bar_height(line))
}

pub fn draw(ctx: &mut Ctx, rect: Rect, state: &ShellState) {
    let focused = state.focus == PanelFocus::Terminal && state.shell_focused;
    ctx.frame(rect, None);
    ctx.hits.push(rect, HitTarget::TerminalArea);
    let line = ctx.line();
    let tab_h = tab_bar_height(line);
    let inner = rect.inset(8.0);
    let tabs = Rect::new(inner.x, inner.y, inner.w, tab_h);
    draw_tabs(ctx, tabs, state, focused);
    let grid = inner.below(tab_h);
    draw_grid(ctx, grid, state, focused);
}

fn draw_tabs(ctx: &mut Ctx, rect: Rect, state: &ShellState, focused: bool) {
    let t = ctx.theme;
    let size = ctx.small();
    let line = ctx.line();
    let cw = ctx.metrics.cell_w * 0.9;
    let mut x = rect.x;
    let title_w = |title: &str| ((title.chars().count().min(24) as f32) * cw).round() + 34.0;
    for (i, tab) in state.terminal.tabs.iter().enumerate() {
        let label: String = format!(
            "{} {}",
            i + 1,
            tab.title.chars().take(24).collect::<String>()
        );
        let w = title_w(&label).min(rect.w * 0.4);
        if x + w > rect.right() - 30.0 {
            break;
        }
        let r = Rect::new(x, rect.y, w, rect.h - 2.0);
        let (fill, fg) = if tab.active {
            (
                with_alpha(t.border, if focused { 0.3 } else { 0.18 }),
                t.text_primary,
            )
        } else {
            (with_alpha(t.border, 0.06), t.text_secondary)
        };
        ctx.scene.fill(r, fill);
        if tab.active {
            ctx.scene
                .fill(Rect::new(r.x, r.bottom() - 2.0, r.w, 2.0), t.border);
        }
        let fg = if tab.exited { t.text_dim } else { fg };
        ctx.scene.text_aligned(
            Rect::new(r.x + 6.0, r.y + (r.h - line) / 2.0, r.w - 26.0, line),
            size,
            fg,
            Align::Left,
            label,
        );
        ctx.hits.push(r, HitTarget::TerminalTab(i));
        let close = Rect::new(r.right() - 20.0, r.y, 18.0, r.h);
        ctx.scene.text_aligned(
            Rect::new(close.x, close.y + (close.h - line) / 2.0, close.w, line),
            size,
            t.text_dim,
            Align::Center,
            "×",
        );
        ctx.hits.push(close, HitTarget::TerminalTabClose(i));
        x += w + 4.0;
    }
    let plus = Rect::new(x, rect.y, 26.0, rect.h - 2.0);
    ctx.scene.fill(plus, with_alpha(t.border, 0.06));
    ctx.scene.text_aligned(
        Rect::new(plus.x, plus.y + (plus.h - line) / 2.0, plus.w, line),
        size,
        t.border,
        Align::Center,
        "+",
    );
    ctx.hits.push(plus, HitTarget::TerminalNewTab);
    ctx.scene.hline(
        rect.x,
        rect.bottom() - 1.0,
        rect.w,
        with_alpha(t.border, 0.35),
    );

    // Right side: scrollback indicator / focus hint
    let frame = &state.terminal.frame;
    let hint = if frame.display_offset > 0 {
        format!("↑ {} lines", frame.display_offset)
    } else if !focused {
        "click to focus".to_string()
    } else {
        String::new()
    };
    if !hint.is_empty() {
        let w = (hint.chars().count() as f32 * cw).round() + 8.0;
        ctx.scene.text_aligned(
            Rect::new(rect.right() - w, rect.y + (rect.h - line) / 2.0, w, line),
            size,
            t.text_dim,
            Align::Right,
            hint,
        );
    }
}

fn draw_grid(ctx: &mut Ctx, rect: Rect, state: &ShellState, focused: bool) {
    let t = ctx.theme;
    let frame = &state.terminal.frame;
    let cw = ctx.metrics.cell_w;
    let ch = ctx.metrics.cell_h;
    let font = ctx.metrics.term_font;
    ctx.scene.fill(rect, t.terminal_bg);
    if frame.lines.is_empty() {
        ctx.scene.text(
            Rect::new(rect.x + 4.0, rect.y + 4.0, rect.w, ch),
            font,
            t.text_dim,
            "$ waiting for shell…",
        );
        return;
    }
    let rows = frame
        .lines
        .len()
        .min(((rect.h / ch).floor()).max(0.0) as usize);
    for (row, line) in frame.lines.iter().take(rows).enumerate() {
        let y = rect.y + row as f32 * ch;
        // Background runs
        let mut run_start = 0usize;
        let mut run_bg = None;
        let flush = |ctx: &mut Ctx, start: usize, end: usize, bg: Option<[f32; 4]>| {
            if let Some(bg) = bg {
                if bg != t.terminal_bg && end > start {
                    let x = rect.x + start as f32 * cw;
                    ctx.scene
                        .fill(Rect::new(x, y, (end - start) as f32 * cw, ch), bg);
                }
            }
        };
        for (col, cell) in line.cells.iter().enumerate() {
            let bg = Some(cell.style.bg);
            if bg != run_bg {
                flush(ctx, run_start, col, run_bg);
                run_start = col;
                run_bg = bg;
            }
        }
        flush(ctx, run_start, line.cells.len(), run_bg);

        // Selection
        for (srow, c0, c1) in &frame.selection {
            if *srow == row {
                let x = rect.x + *c0 as f32 * cw;
                ctx.scene
                    .fill(Rect::new(x, y, (c1 - c0 + 1) as f32 * cw, ch), t.selection);
            }
        }

        // Text spans grouped by style
        let mut spans: Vec<TextSpan> = Vec::new();
        for (col, cell) in line.cells.iter().enumerate() {
            let mut color = cell.style.fg;
            if cell.style.dim {
                color = with_alpha(color, 0.6);
            }
            let text = if cell.text.is_empty() {
                " ".to_string()
            } else {
                cell.text.clone()
            };
            match spans.last_mut() {
                Some(last)
                    if last.color == color
                        && last.bold == cell.style.bold
                        && last.italic == cell.style.italic =>
                {
                    last.text.push_str(&text);
                }
                _ => spans.push(TextSpan {
                    text,
                    color,
                    bold: cell.style.bold,
                    italic: cell.style.italic,
                }),
            }
            if cell.style.underline {
                ctx.scene.fill(
                    Rect::new(rect.x + col as f32 * cw, y + ch - 2.0, cw, 1.0),
                    cell.style.fg,
                );
            }
            if cell.style.strikeout {
                ctx.scene.fill(
                    Rect::new(rect.x + col as f32 * cw, y + ch / 2.0, cw, 1.0),
                    cell.style.fg,
                );
            }
        }
        let trailing_blank = spans.iter().all(|s| s.text.trim().is_empty());
        if !trailing_blank {
            ctx.scene
                .spans(Rect::new(rect.x, y, rect.w, ch), font, ch, spans);
        }
    }

    // Cursor
    if let Some(cur) = frame.cursor {
        if cur.row < rows && cur.shape != CursorShape::Hidden {
            let x = rect.x + cur.col as f32 * cw;
            let y = rect.y + cur.row as f32 * ch;
            let color = if focused {
                t.cursor
            } else {
                with_alpha(t.cursor, 0.5)
            };
            match cur.shape {
                CursorShape::Block => {
                    if focused {
                        ctx.scene.fill(Rect::new(x, y, cw, ch), color);
                        if let Some(cell) =
                            frame.lines.get(cur.row).and_then(|l| l.cells.get(cur.col))
                        {
                            if !cell.text.is_empty() {
                                ctx.scene.text(
                                    Rect::new(x, y, cw * 2.0, ch),
                                    font,
                                    t.terminal_bg,
                                    cell.text.clone(),
                                );
                            }
                        }
                    } else {
                        ctx.scene.stroke(Rect::new(x, y, cw, ch), color, 1.0);
                    }
                }
                CursorShape::Underline => {
                    ctx.scene.fill(Rect::new(x, y + ch - 2.0, cw, 2.0), color)
                }
                CursorShape::Beam => ctx.scene.fill(Rect::new(x, y, 2.0, ch), color),
                CursorShape::Hidden => {}
            }
        }
    }

    if let Some(code) = frame.exited {
        let msg = format!("[process exited with code {code}]  press Enter to close, or Ctrl+Shift+T for a new tab");
        let r = Rect::new(rect.x, rect.bottom() - ch - 4.0, rect.w, ch + 4.0);
        ctx.scene.fill(r, with_alpha(t.warning, 0.2));
        ctx.scene.text(
            Rect::new(r.x + 4.0, r.y + 2.0, r.w, ch),
            font,
            t.warning,
            msg,
        );
    }
    if frame.bell {
        ctx.scene.stroke(rect, t.warning, 2.0);
    }
}
