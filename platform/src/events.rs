//! Events delivered from the platform layer to the application.

use crate::{OutputId, SurfaceId};

/// Keyboard modifier state at the time of an input event.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Modifiers {
    pub ctrl: bool,
    pub alt: bool,
    pub shift: bool,
    pub logo: bool,
    pub caps_lock: bool,
    pub num_lock: bool,
}

/// A keyboard key event (press, repeat or release).
#[derive(Clone, Debug)]
pub struct KeyInput {
    /// Raw xkb keysym value.
    pub keysym: u32,
    /// Raw evdev keycode.
    pub raw_code: u32,
    /// UTF-8 text produced by the key (only on press/repeat).
    pub text: Option<String>,
    pub pressed: bool,
    pub repeat: bool,
    pub modifiers: Modifiers,
}

/// Mouse buttons as Linux input event codes.
pub mod button {
    pub const LEFT: u32 = 0x110;
    pub const RIGHT: u32 = 0x111;
    pub const MIDDLE: u32 = 0x112;
}

/// Geometry and identity of an output.
#[derive(Clone, Debug, PartialEq)]
pub struct OutputInfo {
    pub id: OutputId,
    pub name: Option<String>,
    pub description: Option<String>,
    pub make: String,
    pub model: String,
    pub logical_position: (i32, i32),
    pub logical_size: (i32, i32),
    pub scale_factor: i32,
    pub physical_size_mm: (i32, i32),
    pub refresh_mhz: Option<i32>,
}

/// Events the application receives after each dispatch of the event loop.
#[derive(Debug)]
pub enum PlatformEvent<E> {
    OutputAdded(OutputInfo),
    OutputChanged(OutputInfo),
    OutputRemoved(OutputId),
    /// The window list of `zwlr_foreign_toplevel_management_v1` changed (see
    /// [`crate::Platform::toplevels`]).
    ToplevelsChanged,
    /// The compositor configured a surface: the size is in logical pixels, the scale is
    /// the fractional (or integer) scale to render at.
    Configure {
        surface: SurfaceId,
        width: u32,
        height: u32,
        scale: f64,
    },
    /// The surface's preferred scale changed without a size change.
    ScaleChanged {
        surface: SurfaceId,
        scale: f64,
    },
    /// A previously requested frame callback fired: the surface may render again.
    Frame {
        surface: SurfaceId,
    },
    KeyboardEnter {
        surface: SurfaceId,
    },
    KeyboardLeave {
        surface: SurfaceId,
    },
    Key {
        surface: SurfaceId,
        key: KeyInput,
    },
    ModifiersChanged {
        modifiers: Modifiers,
    },
    PointerEnter {
        surface: SurfaceId,
        x: f64,
        y: f64,
    },
    PointerLeave {
        surface: SurfaceId,
    },
    PointerMotion {
        surface: SurfaceId,
        x: f64,
        y: f64,
    },
    PointerButton {
        surface: SurfaceId,
        button: u32,
        pressed: bool,
        x: f64,
        y: f64,
    },
    /// Scroll in logical pixels (positive = down / right).
    PointerAxis {
        surface: SurfaceId,
        dx: f64,
        dy: f64,
        x: f64,
        y: f64,
    },
    /// The compositor closed the surface (layer surface closed or window close request).
    Closed {
        surface: SurfaceId,
    },
    /// Clipboard contents arrived in response to `request_paste`.
    Paste {
        text: String,
        primary: bool,
    },
    /// An application-defined event pushed through a calloop source.
    App(E),
}
