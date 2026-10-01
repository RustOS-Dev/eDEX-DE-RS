use std::{cell::RefCell, os::unix::io::OwnedFd};

use smithay::{
    desktop::{space::SpaceElement, Window},
    input::pointer::Focus,
    utils::{Logical, Rectangle, SERIAL_COUNTER},
    wayland::{
        compositor::with_states,
        selection::{
            data_device::{
                clear_data_device_selection, current_data_device_selection_userdata,
                request_data_device_client_selection, set_data_device_selection,
            },
            primary_selection::{
                clear_primary_selection, current_primary_selection_userdata,
                request_primary_client_selection, set_primary_selection,
            },
            SelectionTarget,
        },
        xwayland_shell::{XWaylandShellHandler, XWaylandShellState},
    },
    xwayland::{
        xwm::{Reorder, ResizeEdge as X11ResizeEdge, WmWindowProperty, XwmId},
        X11Surface, X11Wm, XwmHandler,
    },
};
use tracing::{error, trace};

use crate::{focus::KeyboardFocusTarget, state::Backend, EdexState};

use crate::wm::WindowInfo;
use comp_proto::WindowMode;

use super::{
    PointerMoveSurfaceGrab, PointerResizeSurfaceGrab, ResizeData, ResizeState, SurfaceData,
    TouchMoveSurfaceGrab, WindowElement,
};

impl<BackendData: Backend> XWaylandShellHandler for EdexState<BackendData> {
    fn xwayland_shell_state(&mut self) -> &mut XWaylandShellState {
        &mut self.xwayland_shell_state
    }
}

impl<BackendData: Backend> XwmHandler for EdexState<BackendData> {
    fn xwm_state(&mut self, _xwm: XwmId) -> &mut X11Wm {
        self.xwm.as_mut().unwrap()
    }

    fn new_window(&mut self, _xwm: XwmId, _window: X11Surface) {}
    fn new_override_redirect_window(&mut self, _xwm: XwmId, _window: X11Surface) {}

    fn map_window_request(&mut self, _xwm: XwmId, window: X11Surface) {
        if let Err(e) = window.set_mapped(true) {
            tracing::warn!("cannot map X11 window: {e}");
            return;
        }
        let fixed = matches!((window.min_size(), window.max_size()), (Some(a), Some(b)) if a == b && !a.is_empty());
        let info = WindowInfo {
            app_id: window.class(),
            title: window.title(),
            xwayland: true,
            pid: window.pid().map(|p| p as i32),
            wants_float: window.is_transient_for().is_some() || window.is_popup() || fixed,
            preferred: Some((window.geometry().size.w, window.geometry().size.h))
                .filter(|(w, h)| *w > 0 && *h > 0),
        };
        let element = WindowElement(Window::new_x11_window(window));
        self.manage_window(element, info);
        self.flush();
    }

    fn mapped_override_redirect_window(&mut self, _xwm: XwmId, window: X11Surface) {
        // Menus and tooltips place themselves.
        let location = window.geometry().loc;
        let window = WindowElement(Window::new_x11_window(window));
        self.space.map_element(window, location, true);
    }

    fn unmapped_window(&mut self, _xwm: XwmId, window: X11Surface) {
        let element = self.x11_element(&window);
        if let Some(elem) = element {
            if self.window_id(&elem).is_some() {
                self.unmanage_window(&elem);
            } else {
                self.space.unmap_elem(&elem);
            }
        }
        if !window.is_override_redirect() {
            let _ = window.set_mapped(false);
        }
    }

    fn destroyed_window(&mut self, _xwm: XwmId, window: X11Surface) {
        if let Some(elem) = self.x11_element(&window) {
            self.unmanage_window(&elem);
        }
    }

    fn configure_request(
        &mut self,
        _xwm: XwmId,
        window: X11Surface,
        x: Option<i32>,
        y: Option<i32>,
        w: Option<u32>,
        h: Option<u32>,
        _reorder: Option<Reorder>,
    ) {
        let managed = self.x11_element(&window).and_then(|e| self.window_id(&e));
        match managed {
            // Tiled windows keep their tile: answer with the geometry they already have.
            Some(id) if !self.wm.is_floating(id) => {
                let _ = window.configure(None);
            }
            _ => {
                let mut geo = window.geometry();
                if let Some(x) = x.filter(|_| managed.is_none()) {
                    geo.loc.x = x;
                }
                if let Some(y) = y.filter(|_| managed.is_none()) {
                    geo.loc.y = y;
                }
                if let Some(w) = w {
                    geo.size.w = w as i32;
                }
                if let Some(h) = h {
                    geo.size.h = h as i32;
                }
                let _ = window.configure(geo);
            }
        }
    }

    fn configure_notify(
        &mut self,
        _xwm: XwmId,
        window: X11Surface,
        geometry: Rectangle<i32, Logical>,
        _above: Option<u32>,
    ) {
        // Only override-redirect windows position themselves.
        if !window.is_override_redirect() {
            return;
        }
        let Some(elem) = self.x11_element(&window) else {
            return;
        };
        self.space.map_element(elem, geometry.loc, false);
    }

    fn property_notify(&mut self, _xwm: XwmId, window: X11Surface, _property: WmWindowProperty) {
        if let Some(elem) = self.x11_element(&window) {
            self.refresh_window_info(&elem);
        }
    }

    fn maximize_request(&mut self, _xwm: XwmId, window: X11Surface) {
        self.x11_mode_request(&window, WindowMode::Maximized, true);
    }

    fn unmaximize_request(&mut self, _xwm: XwmId, window: X11Surface) {
        self.x11_mode_request(&window, WindowMode::Maximized, false);
    }

    fn fullscreen_request(&mut self, _xwm: XwmId, window: X11Surface) {
        self.x11_mode_request(&window, WindowMode::Fullscreen, true);
    }

    fn unfullscreen_request(&mut self, _xwm: XwmId, window: X11Surface) {
        self.x11_mode_request(&window, WindowMode::Fullscreen, false);
    }

    fn minimize_request(&mut self, _xwm: XwmId, window: X11Surface) {
        if let Some(id) = self.x11_element(&window).and_then(|e| self.window_id(&e)) {
            self.wm.minimize(Some(id));
            self.mark_layout();
        }
    }

    fn resize_request(
        &mut self,
        _xwm: XwmId,
        window: X11Surface,
        _button: u32,
        edges: X11ResizeEdge,
    ) {
        if !self.x11_floating(&window) {
            return;
        }
        let Some(start_data) = self.pointer.grab_start_data() else {
            return;
        };

        let Some(element) = self
            .space
            .elements()
            .find(|e| matches!(e.0.x11_surface(), Some(w) if w == &window))
        else {
            return;
        };

        let geometry = element.geometry();
        let loc = self.space.element_location(element).unwrap();
        let (initial_window_location, initial_window_size) = (loc, geometry.size);

        with_states(&element.wl_surface().unwrap(), move |states| {
            states
                .data_map
                .get::<RefCell<SurfaceData>>()
                .unwrap()
                .borrow_mut()
                .resize_state = ResizeState::Resizing(ResizeData {
                edges: edges.into(),
                initial_window_location,
                initial_window_size,
            });
        });

        let grab = PointerResizeSurfaceGrab {
            start_data,
            window: element.clone(),
            edges: edges.into(),
            initial_window_location,
            initial_window_size,
            last_window_size: initial_window_size,
        };

        let pointer = self.pointer.clone();
        pointer.set_grab(self, grab, SERIAL_COUNTER.next_serial(), Focus::Clear);
    }

    fn move_request(&mut self, _xwm: XwmId, window: X11Surface, _button: u32) {
        self.move_request_x11(&window)
    }

    fn allow_selection_access(&mut self, xwm: XwmId, _selection: SelectionTarget) -> bool {
        if let Some(keyboard) = self.seat.get_keyboard() {
            // check that an X11 window is focused
            if let Some(KeyboardFocusTarget::Window(w)) = keyboard.current_focus() {
                if let Some(surface) = w.x11_surface() {
                    if surface.xwm_id().unwrap() == xwm {
                        return true;
                    }
                }
            }
        }
        false
    }

    fn send_selection(
        &mut self,
        _xwm: XwmId,
        selection: SelectionTarget,
        mime_type: String,
        fd: OwnedFd,
    ) {
        match selection {
            SelectionTarget::Clipboard => {
                if let Err(err) = request_data_device_client_selection(&self.seat, mime_type, fd) {
                    error!(
                        ?err,
                        "Failed to request current wayland clipboard for Xwayland",
                    );
                }
            }
            SelectionTarget::Primary => {
                if let Err(err) = request_primary_client_selection(&self.seat, mime_type, fd) {
                    error!(
                        ?err,
                        "Failed to request current wayland primary selection for Xwayland",
                    );
                }
            }
        }
    }

    fn new_selection(&mut self, _xwm: XwmId, selection: SelectionTarget, mime_types: Vec<String>) {
        trace!(?selection, ?mime_types, "Got Selection from X11",);
        // TODO check, that focused windows is X11 window before doing this
        match selection {
            SelectionTarget::Clipboard => {
                set_data_device_selection(&self.display_handle, &self.seat, mime_types, ())
            }
            SelectionTarget::Primary => {
                set_primary_selection(&self.display_handle, &self.seat, mime_types, ())
            }
        }
    }

    fn cleared_selection(&mut self, _xwm: XwmId, selection: SelectionTarget) {
        match selection {
            SelectionTarget::Clipboard => {
                if current_data_device_selection_userdata(&self.seat).is_some() {
                    clear_data_device_selection(&self.display_handle, &self.seat)
                }
            }
            SelectionTarget::Primary => {
                if current_primary_selection_userdata(&self.seat).is_some() {
                    clear_primary_selection(&self.display_handle, &self.seat)
                }
            }
        }
    }
}

impl<BackendData: Backend> EdexState<BackendData> {
    fn x11_element(&self, window: &X11Surface) -> Option<WindowElement> {
        self.wm
            .handles()
            .map(|(_, w)| w)
            .find(|e| matches!(e.0.x11_surface(), Some(w) if w == window))
            .cloned()
            .or_else(|| {
                self.space
                    .elements()
                    .find(|e| matches!(e.0.x11_surface(), Some(w) if w == window))
                    .cloned()
            })
    }

    fn x11_floating(&self, window: &X11Surface) -> bool {
        self.x11_element(window)
            .and_then(|e| self.window_id(&e))
            .is_some_and(|id| self.wm.is_floating(id))
    }

    fn x11_mode_request(&mut self, window: &X11Surface, mode: WindowMode, on: bool) {
        if let Some(id) = self.x11_element(window).and_then(|e| self.window_id(&e)) {
            self.wm.request_mode(id, mode, on);
            self.mark_layout();
        }
    }

    pub fn move_request_x11(&mut self, window: &X11Surface) {
        if !self.x11_floating(window) {
            return;
        }
        let Some(element) = self.x11_element(window) else {
            return;
        };
        let Some(initial_window_location) = self.space.element_location(&element) else {
            return;
        };
        if let Some(touch) = self.seat.get_touch() {
            if let Some(start_data) = touch.grab_start_data() {
                let grab = TouchMoveSurfaceGrab {
                    start_data,
                    window: element,
                    initial_window_location,
                };
                touch.set_grab(self, grab, SERIAL_COUNTER.next_serial());
                return;
            }
        }
        let Some(start_data) = self.pointer.grab_start_data() else {
            return;
        };
        let grab = PointerMoveSurfaceGrab {
            start_data,
            window: element,
            initial_window_location,
        };
        let pointer = self.pointer.clone();
        pointer.set_grab(self, grab, SERIAL_COUNTER.next_serial(), Focus::Clear);
    }
}
