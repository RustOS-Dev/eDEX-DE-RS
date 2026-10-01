#![warn(rust_2018_idioms)]
// If no backend is enabled, a large portion of the codebase is unused.
// So silence this useless warning for the CI.

pub mod binds;
pub mod config;
pub mod control;
pub mod cursor;
pub mod drawing;
pub mod edid;
pub mod focus;
pub mod gamma;
pub mod input_handler;
pub mod ipc;
pub mod lifecycle;
pub mod manage;
pub mod render;
pub mod screenshot;
pub mod session;
pub mod shell;
pub mod state;
pub mod udev;
pub mod winit;
pub mod wm;

pub use state::{ClientState, EdexState};
