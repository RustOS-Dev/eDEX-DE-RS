use std::{borrow::Cow, cell::RefCell, time::Duration};

use smithay::{
    backend::renderer::{
        element::{
            solid::{SolidColorBuffer, SolidColorRenderElement},
            surface::WaylandSurfaceRenderElement,
            AsRenderElements, Kind,
        },
        ImportAll, ImportMem, Renderer, Texture,
    },
    desktop::{
        space::SpaceElement, utils::OutputPresentationFeedback, Window, WindowSurface,
        WindowSurfaceType,
    },
    output::Output,
    reexports::{
        wayland_protocols::wp::presentation_time::server::wp_presentation_feedback,
        wayland_server::protocol::wl_surface::WlSurface,
    },
    render_elements,
    utils::{user_data::UserDataMap, IsAlive, Logical, Physical, Point, Rectangle, Scale},
    wayland::{
        compositor::SurfaceData as WlSurfaceData, dmabuf::DmabufFeedback, seat::WaylandFocus,
    },
};

use crate::focus::PointerFocusTarget;

#[derive(Debug, Clone, PartialEq)]
pub struct WindowElement(pub Window);

impl WindowElement {
    pub fn surface_under(
        &self,
        location: Point<f64, Logical>,
        window_type: WindowSurfaceType,
    ) -> Option<(PointerFocusTarget, Point<i32, Logical>)> {
        let surface_under = self.0.surface_under(location, window_type);
        match self.0.underlying_surface() {
            WindowSurface::Wayland(_) => {
                surface_under.map(|(surface, loc)| (PointerFocusTarget::WlSurface(surface), loc))
            }
            WindowSurface::X11(s) => {
                surface_under.map(|(_, loc)| (PointerFocusTarget::X11Surface(s.clone()), loc))
            }
        }
    }

    /// Set the focus border drawn around the window (`width` 0 = none).
    pub fn set_border(&self, width: i32, color: [f32; 4]) {
        let data = self.user_data();
        data.insert_if_missing(|| RefCell::new(Border::default()));
        let mut b = data.get::<RefCell<Border>>().unwrap().borrow_mut();
        b.width = width.max(0);
        b.color = color;
    }

    fn border(&self) -> (i32, [f32; 4]) {
        self.user_data()
            .get::<RefCell<Border>>()
            .map(|b| {
                let b = b.borrow();
                (b.width, b.color)
            })
            .unwrap_or((0, [0.0; 4]))
    }

    pub fn with_surfaces<F>(&self, processor: F)
    where
        F: FnMut(&WlSurface, &WlSurfaceData),
    {
        self.0.with_surfaces(processor);
    }

    pub fn send_frame<T, F>(
        &self,
        output: &Output,
        time: T,
        throttle: Option<Duration>,
        primary_scan_out_output: F,
    ) where
        T: Into<Duration>,
        F: FnMut(&WlSurface, &WlSurfaceData) -> Option<Output> + Copy,
    {
        self.0
            .send_frame(output, time, throttle, primary_scan_out_output)
    }

    pub fn send_dmabuf_feedback<'a, P, F>(
        &self,
        output: &Output,
        primary_scan_out_output: P,
        select_dmabuf_feedback: F,
    ) where
        P: FnMut(&WlSurface, &WlSurfaceData) -> Option<Output> + Copy,
        F: Fn(&WlSurface, &WlSurfaceData) -> &'a DmabufFeedback + Copy,
    {
        self.0
            .send_dmabuf_feedback(output, primary_scan_out_output, select_dmabuf_feedback)
    }

    pub fn take_presentation_feedback<F1, F2>(
        &self,
        output_feedback: &mut OutputPresentationFeedback,
        primary_scan_out_output: F1,
        presentation_feedback_flags: F2,
    ) where
        F1: FnMut(&WlSurface, &WlSurfaceData) -> Option<Output> + Copy,
        F2: FnMut(&WlSurface, &WlSurfaceData) -> wp_presentation_feedback::Kind + Copy,
    {
        self.0.take_presentation_feedback(
            output_feedback,
            primary_scan_out_output,
            presentation_feedback_flags,
        )
    }

    #[inline]
    pub fn is_x11(&self) -> bool {
        self.0.is_x11()
    }

    #[inline]
    pub fn is_wayland(&self) -> bool {
        self.0.is_wayland()
    }

    #[inline]
    pub fn wl_surface(&self) -> Option<Cow<'_, WlSurface>> {
        self.0.wl_surface()
    }

    #[inline]
    pub fn user_data(&self) -> &UserDataMap {
        self.0.user_data()
    }
}

impl IsAlive for WindowElement {
    #[inline]
    fn alive(&self) -> bool {
        self.0.alive()
    }
}

impl SpaceElement for WindowElement {
    fn geometry(&self) -> Rectangle<i32, Logical> {
        SpaceElement::geometry(&self.0)
    }
    fn bbox(&self) -> Rectangle<i32, Logical> {
        let mut bbox = SpaceElement::bbox(&self.0);
        let (width, _) = self.border();
        if width > 0 {
            let geo = SpaceElement::geometry(&self.0);
            let outer = Rectangle::new(
                geo.loc - Point::from((width, width)),
                (geo.size.w + 2 * width, geo.size.h + 2 * width).into(),
            );
            bbox = bbox.merge(outer);
        }
        bbox
    }
    fn is_in_input_region(&self, point: &Point<f64, Logical>) -> bool {
        SpaceElement::is_in_input_region(&self.0, point)
    }
    fn z_index(&self) -> u8 {
        SpaceElement::z_index(&self.0)
    }

    fn set_activate(&self, activated: bool) {
        SpaceElement::set_activate(&self.0, activated);
    }
    fn output_enter(&self, output: &Output, overlap: Rectangle<i32, Logical>) {
        SpaceElement::output_enter(&self.0, output, overlap);
    }
    fn output_leave(&self, output: &Output) {
        SpaceElement::output_leave(&self.0, output);
    }
    fn refresh(&self) {
        SpaceElement::refresh(&self.0);
    }
}

/// The four edges of a window's focus border, kept between frames so damage tracking sees
/// unchanged buffers.
#[derive(Debug, Default)]
struct Border {
    width: i32,
    color: [f32; 4],
    buffers: [SolidColorBuffer; 4],
}

render_elements!(
    pub WindowRenderElement<R> where R: ImportAll + ImportMem;
    Window=WaylandSurfaceRenderElement<R>,
    Decoration=SolidColorRenderElement,
);

impl<R: Renderer> std::fmt::Debug for WindowRenderElement<R> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Window(arg0) => f.debug_tuple("Window").field(arg0).finish(),
            Self::Decoration(arg0) => f.debug_tuple("Decoration").field(arg0).finish(),
            Self::_GenericCatcher(arg0) => f.debug_tuple("_GenericCatcher").field(arg0).finish(),
        }
    }
}

impl<R> AsRenderElements<R> for WindowElement
where
    R: Renderer + ImportAll + ImportMem,
    R::TextureId: Clone + Texture + 'static,
{
    type RenderElement = WindowRenderElement<R>;

    fn render_elements<C: From<Self::RenderElement>>(
        &self,
        renderer: &mut R,
        location: Point<i32, Physical>,
        scale: Scale<f64>,
        alpha: f32,
    ) -> Vec<C> {
        let mut out: Vec<C> =
            AsRenderElements::render_elements(&self.0, renderer, location, scale, alpha)
                .into_iter()
                .map(C::from)
                .collect();
        let Some(cell) = self.user_data().get::<RefCell<Border>>() else {
            return out;
        };
        let mut border = cell.borrow_mut();
        if border.width <= 0 {
            return out;
        }
        // `location` is where the window's surface origin lands; the border hugs the window
        // geometry, which client-side shadows can offset inside the surface.
        let geo = SpaceElement::geometry(&self.0);
        let (w, h, b) = (geo.size.w, geo.size.h, border.width);
        let edges: [(i32, i32, i32, i32); 4] = [
            (-b, -b, w + 2 * b, b),
            (-b, h, w + 2 * b, b),
            (-b, 0, b, h),
            (w, 0, b, h),
        ];
        let color = border.color;
        for (buffer, (x, y, ew, eh)) in border.buffers.iter_mut().zip(edges) {
            buffer.update((ew, eh), color);
            let loc = location
                + (geo.loc + Point::<i32, Logical>::from((x, y))).to_physical_precise_round(scale);
            out.push(C::from(WindowRenderElement::Decoration(
                SolidColorRenderElement::from_buffer(buffer, loc, scale, alpha, Kind::Unspecified),
            )));
        }
        out
    }
}
