//! Surface bookkeeping shared by canvases, reservers, overlays, toasts and xdg windows.

use smithay_client_toolkit::{
    shell::{wlr_layer::LayerSurface, xdg::window::Window},
    shm::slot::Buffer,
};
use wayland_client::protocol::wl_surface::WlSurface;
use wayland_protocols::wp::{
    fractional_scale::v1::client::wp_fractional_scale_v1::WpFractionalScaleV1,
    viewporter::client::wp_viewport::WpViewport,
};

use crate::OutputId;

/// Which edge a reserver surface occupies.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Edge {
    Top,
    Bottom,
    Left,
    Right,
}

/// The role a surface plays in the shell.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SurfaceRole {
    /// Full-output background canvas drawn with wgpu.
    Canvas,
    /// Invisible layer surface that only reserves an exclusive zone.
    Reserver(Edge),
    /// Full-output overlay drawn with wgpu (launcher, settings, ...).
    Overlay,
    /// Small overlay-layer surface for toasts and OSDs.
    Toast,
    /// xdg-toplevel window (used by the greeter under cage).
    Window,
}

pub(crate) enum SurfaceKind {
    Layer(LayerSurface),
    #[allow(dead_code)]
    Window(Window),
}

pub(crate) struct SurfaceEntry {
    pub wl_surface: WlSurface,
    pub kind: SurfaceKind,
    pub role: SurfaceRole,
    pub output: Option<OutputId>,
    pub viewport: Option<WpViewport>,
    pub fractional: Option<WpFractionalScaleV1>,
    /// Preferred scale in 1/120ths (fractional-scale-v1), if received.
    pub scale120: Option<u32>,
    /// Integer scale from wl_surface enter/leave, used as a fallback.
    pub int_scale: i32,
    pub logical_size: (u32, u32),
    pub configured: bool,
    pub frame_pending: bool,
    /// The 1x1 buffer used by reservers.
    pub reserver_buffer: Option<Buffer>,
}

impl SurfaceEntry {
    pub fn scale(&self) -> f64 {
        match self.scale120 {
            Some(s) if s > 0 => s as f64 / 120.0,
            _ => self.int_scale.max(1) as f64,
        }
    }

    /// Buffer (pixel) size for the current logical size and scale.
    pub fn buffer_size(&self) -> (u32, u32) {
        let s = self.scale();
        let w = (self.logical_size.0 as f64 * s).round().max(1.0) as u32;
        let h = (self.logical_size.1 as f64 * s).round().max(1.0) as u32;
        (w, h)
    }
}
