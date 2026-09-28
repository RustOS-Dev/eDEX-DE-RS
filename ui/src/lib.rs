//! eDEX-DE user interface model and scene builder (no GPU dependency).

pub mod boot;
pub mod filesystem;
pub mod form;
pub mod geometry;
pub mod hit;
pub mod keyboard;
pub mod layout;
pub mod overlays;
pub mod panels;
pub mod scene;
pub mod shell;
pub mod state;
pub mod terminal_model;
pub mod theme;
pub mod widgets;

pub use geometry::{Color, Rect};
pub use hit::{HitMap, HitTarget};
pub use layout::{LayoutConfig, Metrics, PanelLayout};
pub use scene::{Scene, TextSpan, TextSpec};
pub use state::ShellState;
pub use theme::Theme;
