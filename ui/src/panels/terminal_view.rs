//! Terminal panel: tab bar and cell grid.

use crate::{
    geometry::{with_alpha, Rect},
    hit::HitTarget,
    scene::{Align, TextSpan},
    state::{PanelFocus, ShellState},
    terminal_model::CursorShape,
    widgets::Ctx,
};

pub use crate::layout::tab_bar_height;
use crate::layout::PANEL_INSET;

/// The rectangle the grid is drawn into (used to size the PTY).
pub fn grid_rect(panel: Rect, line: f32) -> Rect {
    panel.inset(PANEL_INSET).below(tab_bar_height(line))
}

pub fn draw(ctx: &mut Ctx, rect: Rect, state: &ShellState) {
    let focused = state.focus == PanelFocus::Terminal && state.shell_focused;
    ctx.frame(rect, None);
    ctx.hits.push(rect, HitTarget::TerminalArea);
    let grid = rect.inset(PANEL_INSET).below(tab_bar_height(ctx.line()));
    if state.apps_cover_terminal {
        // Hidden behind application windows; drawing it would only show through the gaps.
        ctx.scene.fill(grid, ctx.theme.terminal_bg);
    } else {
        draw_grid(ctx, grid, state, focused);
    }
}

/// Width of the window-control buttons at the right end of the strip.
fn control_w(h: f32) -> f32 {
    (h * 1.25).round()
}

/// The centre tab strip: terminal tabs, `+`, application windows (minimized ones dimmed), and
/// minimize / maximize / close for the focused window. Drawn on its own surface above windows.
pub fn draw_tab_strip(ctx: &mut Ctx, rect: Rect, state: &ShellState) {
    let t = ctx.theme;
    let size = ctx.small();
    let line = ctx.line();
    let cw = ctx.metrics.cell_w * 0.9;
    let focused = state.focus == PanelFocus::Terminal && state.shell_focused;
    let active_app = state.windows.iter().position(|w| w.active && !w.minimized);
    let controls = if active_app.is_some() {
        control_w(rect.h) * 3.0 + 6.0
    } else {
        0.0
    };
    let limit = rect.right() - controls;
    let label_w = |chars: usize| ((chars.min(24) as f32) * cw).round() + 34.0;
    let n_tabs = state.terminal.tabs.len() + state.windows.len();
    // Shrink tabs evenly when they do not all fit at their natural width.
    let fair = ((limit - rect.x - 34.0 - 8.0) / n_tabs.max(1) as f32 - 4.0).max(56.0);
    let text_mid = |r: Rect| Rect::new(r.x + 6.0, r.y + (r.h - line) / 2.0, r.w - 26.0, line);
    let mut x = rect.x;

    let apps_in_front = state.apps_cover_terminal && active_app.is_some();
    for (i, tab) in state.terminal.tabs.iter().enumerate() {
        let label = format!(
            "{} {}",
            i + 1,
            tab.title.chars().take(24).collect::<String>()
        );
        let w = label_w(label.chars().count()).min(fair);
        if x + w > limit - 30.0 {
            break;
        }
        let r = Rect::new(x, rect.y, w, rect.h - 2.0);
        let selected = tab.active && !apps_in_front;
        let (fill, fg) = if selected {
            (
                with_alpha(t.border, if focused { 0.3 } else { 0.18 }),
                t.text_primary,
            )
        } else {
            (with_alpha(t.border, 0.06), t.text_secondary)
        };
        ctx.scene.fill(r, fill);
        if selected {
            ctx.scene
                .fill(Rect::new(r.x, r.bottom() - 2.0, r.w, 2.0), t.border);
        }
        let fg = if tab.exited { t.text_dim } else { fg };
        ctx.scene
            .text_aligned(text_mid(r), size, fg, Align::Left, label);
        ctx.hits.push(r, HitTarget::TerminalTab(i));
        close_mark(ctx, r, HitTarget::TerminalTabClose(i));
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
    x += 26.0 + 4.0;

    if !state.windows.is_empty() {
        ctx.scene.fill(
            Rect::new(x, rect.y + 4.0, 1.0, rect.h - 10.0),
            with_alpha(t.border, 0.5),
        );
        x += 5.0;
    }
    for (i, win) in state.windows.iter().enumerate() {
        let name = if win.title.trim().is_empty() {
            &win.class
        } else {
            &win.title
        };
        let mark = if win.minimized { "▾ " } else { "" };
        let label = format!("{mark}{}", name.chars().take(22).collect::<String>());
        let w = label_w(label.chars().count()).min(fair);
        if x + w > limit {
            break;
        }
        let r = Rect::new(x, rect.y, w, rect.h - 2.0);
        let (fill, fg) = if win.active {
            (with_alpha(t.accent, 0.22), t.text_primary)
        } else if win.minimized {
            ([0.0; 4], t.text_dim)
        } else {
            (with_alpha(t.accent, 0.07), t.text_secondary)
        };
        ctx.scene.fill(r, fill);
        if win.minimized {
            ctx.scene.stroke(r, with_alpha(t.text_dim, 0.5), 1.0);
        }
        if win.active {
            ctx.scene
                .fill(Rect::new(r.x, r.bottom() - 2.0, r.w, 2.0), t.accent);
        }
        ctx.scene
            .text_aligned(text_mid(r), size, fg, Align::Left, label);
        ctx.hits.push(r, HitTarget::AppTab(i));
        close_mark(ctx, r, HitTarget::AppTabClose(i));
        x += w + 4.0;
    }
    ctx.scene.hline(
        rect.x,
        rect.bottom() - 1.0,
        rect.w,
        with_alpha(t.border, 0.35),
    );

    if let Some(i) = active_app {
        let bw = control_w(rect.h);
        let maximized = state.windows[i].maximized;
        let buttons = [
            ("↓", HitTarget::WindowMinimize, t.text_secondary),
            (
                "□",
                HitTarget::WindowMaximize,
                if maximized {
                    t.accent
                } else {
                    t.text_secondary
                },
            ),
            ("×", HitTarget::WindowClose, t.error),
        ];
        let mut bx = rect.right() - bw * 3.0;
        for (glyph, target, color) in buttons {
            let r = Rect::new(bx, rect.y, bw - 2.0, rect.h - 2.0);
            ctx.scene.fill(r, with_alpha(color, 0.12));
            ctx.scene.stroke(r, with_alpha(color, 0.6), 1.0);
            ctx.scene.text_aligned(
                Rect::new(r.x, r.y + (r.h - line) / 2.0, r.w, line),
                size,
                color,
                Align::Center,
                glyph,
            );
            ctx.hits.push(r, target);
            bx += bw;
        }
        return;
    }

    // No app focused: scrollback indicator / focus hint on the right.
    let frame = &state.terminal.frame;
    let hint = if frame.display_offset > 0 {
        format!("↑ {} lines", frame.display_offset)
    } else if !focused && !state.apps_cover_terminal {
        "click to focus".to_string()
    } else {
        String::new()
    };
    if !hint.is_empty() {
        let w = (hint.chars().count() as f32 * cw).round() + 8.0;
        if rect.right() - w > x {
            ctx.scene.text_aligned(
                Rect::new(rect.right() - w, rect.y + (rect.h - line) / 2.0, w, line),
                size,
                t.text_dim,
                Align::Right,
                hint,
            );
        }
    }
}

/// The small × at the right end of a tab.
fn close_mark(ctx: &mut Ctx, tab: Rect, target: HitTarget) {
    let line = ctx.line();
    let close = Rect::new(tab.right() - 20.0, tab.y, 18.0, tab.h);
    ctx.scene.text_aligned(
        Rect::new(close.x, close.y + (close.h - line) / 2.0, close.w, line),
        ctx.small(),
        ctx.theme.text_dim,
        Align::Center,
        "×",
    );
    ctx.hits.push(close, target);
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
