//! Renderer-facing snapshot of a terminal grid, produced by the `terminal` crate.

use crate::geometry::Color;

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct CellStyle {
    pub fg: Color,
    pub bg: Color,
    pub bold: bool,
    pub italic: bool,
    pub underline: bool,
    pub strikeout: bool,
    pub dim: bool,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Cell {
    /// Grapheme in the cell (empty for the trailing half of a wide character).
    pub text: String,
    pub style: CellStyle,
    pub wide: bool,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub enum CursorShape {
    #[default]
    Block,
    Underline,
    Beam,
    Hidden,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Cursor {
    pub col: usize,
    pub row: usize,
    pub shape: CursorShape,
}

#[derive(Clone, Debug, PartialEq)]
pub struct TerminalLine {
    pub cells: Vec<Cell>,
}

/// One tab's visible content.
#[derive(Clone, Debug, PartialEq, Default)]
pub struct TerminalFrame {
    pub cols: usize,
    pub rows: usize,
    pub lines: Vec<TerminalLine>,
    pub cursor: Option<Cursor>,
    /// Selected cells as (row, col) ranges per row.
    pub selection: Vec<(usize, usize, usize)>,
    /// Lines scrolled back from the bottom.
    pub display_offset: usize,
    pub total_lines: usize,
    pub title: String,
    pub bell: bool,
    pub exited: Option<i32>,
}

#[derive(Clone, Debug, PartialEq)]
pub struct TabInfo {
    pub title: String,
    pub active: bool,
    pub exited: bool,
}
