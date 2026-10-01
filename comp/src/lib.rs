//! The shell's side of the compositor: requests to edex-comp and a cached state model kept
//! current by its event stream.

pub mod client;
pub mod model;

pub use client::CompSocket;
pub use comp_proto::{
    Event, EventReader, Monitor as MonitorInfo, Rect, Window, WindowId, WindowMode,
    WorkspaceTarget, MINIMIZED_WORKSPACE,
};
pub use model::CompState;
