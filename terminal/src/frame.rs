//! Convert a `Term` into the renderer-facing `TerminalFrame`.

use alacritty_terminal::{
    event::EventListener,
    grid::Dimensions,
    term::{cell::Flags, Term, TermMode},
    vte::ansi::CursorShape as AlacCursorShape,
};
use ui::terminal_model::{Cell, CellStyle, Cursor, CursorShape, TerminalFrame, TerminalLine};

use crate::colors::Palette;

pub struct FrameOptions {
    pub cursor_visible: bool,
    pub focused: bool,
    pub bell: bool,
    pub exited: Option<i32>,
    pub title: String,
}

pub fn build_frame<L: EventListener>(
    term: &Term<L>,
    palette: &Palette,
    opts: &FrameOptions,
) -> TerminalFrame {
    let content = term.renderable_content();
    let cols = term.columns();
    let rows = term.screen_lines();
    let display_offset = content.display_offset;
    let colors = content.colors;
    let mut lines: Vec<TerminalLine> = (0..rows)
        .map(|_| TerminalLine {
            cells: Vec::with_capacity(cols),
        })
        .collect();
    let mut selection = Vec::new();
    let mut sel_row: Option<(usize, usize, usize)> = None;

    for indexed in content.display_iter {
        let row = (indexed.point.line.0 + display_offset as i32).max(0) as usize;
        if row >= rows {
            continue;
        }
        let col = indexed.point.column.0;
        let cell = &indexed.cell;
        let flags = cell.flags;
        if flags.contains(Flags::WIDE_CHAR_SPACER)
            || flags.contains(Flags::LEADING_WIDE_CHAR_SPACER)
        {
            lines[row].cells.push(Cell {
                text: String::new(),
                style: default_style(palette),
                wide: false,
            });
            continue;
        }
        let bold = flags.intersects(Flags::BOLD);
        let mut fg = palette.resolve(cell.fg, colors, bold);
        let mut bg = palette.resolve(cell.bg, colors, false);
        if flags.contains(Flags::INVERSE) {
            std::mem::swap(&mut fg, &mut bg);
        }
        if flags.contains(Flags::HIDDEN) {
            fg = bg;
        }
        let mut text = String::new();
        if cell.c != ' ' && cell.c != '\0' {
            text.push(cell.c);
        }
        if let Some(zw) = cell.zerowidth() {
            text.extend(zw.iter());
        }
        if text.is_empty()
            && bg == palette.bg
            && !flags.intersects(Flags::ALL_UNDERLINES | Flags::STRIKEOUT)
        {
            // Blank cell with default background: keep it cheap.
            lines[row].cells.push(Cell {
                text: String::new(),
                style: CellStyle {
                    fg,
                    bg,
                    ..default_style(palette)
                },
                wide: false,
            });
        } else {
            lines[row].cells.push(Cell {
                text,
                style: CellStyle {
                    fg,
                    bg,
                    bold,
                    italic: flags.intersects(Flags::ITALIC),
                    underline: flags.intersects(Flags::ALL_UNDERLINES),
                    strikeout: flags.contains(Flags::STRIKEOUT),
                    dim: flags.intersects(Flags::DIM),
                },
                wide: flags.contains(Flags::WIDE_CHAR),
            });
        }
        if let Some(range) = content.selection {
            if range.contains(indexed.point) {
                match sel_row {
                    Some((r, s, e)) if r == row && e + 1 == col => sel_row = Some((r, s, col)),
                    Some(prev) => {
                        selection.push(prev);
                        sel_row = Some((row, col, col));
                    }
                    None => sel_row = Some((row, col, col)),
                }
            }
        }
    }
    if let Some(prev) = sel_row {
        selection.push(prev);
    }

    let mode = content.mode;
    let cursor = if mode.contains(TermMode::SHOW_CURSOR) && opts.exited.is_none() {
        let p = content.cursor.point;
        let row = p.line.0 + display_offset as i32;
        if row >= 0 && (row as usize) < rows {
            let shape = match content.cursor.shape {
                AlacCursorShape::Block => CursorShape::Block,
                AlacCursorShape::Underline => CursorShape::Underline,
                AlacCursorShape::Beam => CursorShape::Beam,
                AlacCursorShape::Hidden => CursorShape::Hidden,
                AlacCursorShape::HollowBlock => CursorShape::Block,
            };
            let shape = if !opts.cursor_visible && opts.focused {
                CursorShape::Hidden
            } else {
                shape
            };
            Some(Cursor {
                col: p.column.0,
                row: row as usize,
                shape,
            })
        } else {
            None
        }
    } else {
        None
    };

    TerminalFrame {
        cols,
        rows,
        lines,
        cursor,
        selection,
        display_offset,
        total_lines: term.grid().total_lines(),
        title: opts.title.clone(),
        bell: opts.bell,
        exited: opts.exited,
    }
}

fn default_style(palette: &Palette) -> CellStyle {
    CellStyle {
        fg: palette.fg,
        bg: palette.bg,
        bold: false,
        italic: false,
        underline: false,
        strikeout: false,
        dim: false,
    }
}
