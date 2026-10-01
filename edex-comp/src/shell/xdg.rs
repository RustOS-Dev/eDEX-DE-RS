use std::cell::RefCell;

use smithay::{
    desktop::{
        find_popup_root_surface, get_popup_toplevel_coords, layer_map_for_output,
        space::SpaceElement, PopupKeyboardGrab, PopupKind, PopupPointerGrab, PopupUngrabStrategy,
        Space, Window, WindowSurfaceType,
    },
    input::{pointer::Focus, Seat},
    reexports::{
        wayland_protocols::xdg::shell::server::xdg_toplevel,
        wayland_server::{
            protocol::{wl_output, wl_seat, wl_surface::WlSurface},
            Resource,
        },
    },
    utils::{Logical, Point, Serial},
    wayland::{
        compositor::{self, with_states},
        seat::WaylandFocus,
        shell::xdg::{
            Configure, PopupSurface, PositionerState, ToplevelSurface, XdgShellHandler,
            XdgShellState, XdgToplevelSurfaceData,
        },
    },
};
use tracing::warn;

use crate::{
    focus::KeyboardFocusTarget,
    shell::{TouchMoveSurfaceGrab, TouchResizeSurfaceGrab},
    state::{Backend, EdexState},
};

use super::{
    PointerMoveSurfaceGrab, PointerResizeSurfaceGrab, ResizeData, ResizeEdge, ResizeState,
    SurfaceData, WindowElement,
};
use crate::wm::WindowInfo;
use comp_proto::WindowMode;

impl<BackendData: Backend> XdgShellHandler for EdexState<BackendData> {
    fn xdg_shell_state(&mut self) -> &mut XdgShellState {
        &mut self.xdg_shell_state
    }

    fn new_toplevel(&mut self, surface: ToplevelSurface) {
        // Do not send a configure here, the initial configure
        // of a xdg_surface has to be sent during the commit if
        // the surface is not already configured
        let window = WindowElement(Window::new_wayland_window(surface.clone()));
        let info = WindowInfo {
            // Dialogs float above their parent.
            wants_float: surface.parent().is_some(),
            pid: self
                .display_handle
                .get_client(surface.wl_surface().id())
                .ok()
                .and_then(|c| c.get_credentials(&self.display_handle).ok())
                .map(|c| c.pid),
            ..Default::default()
        };
        self.manage_window(window, info);
        // Lay out now so the initial configure carries the tiled size.
        self.flush();

        compositor::add_post_commit_hook(surface.wl_surface(), |state: &mut Self, _, surface| {
            handle_toplevel_commit(&mut state.space, surface);
            if let Some(window) = state.window_for_surface(surface) {
                state.first_commit(&window);
                state.refresh_window_info(&window);
            }
        });
    }

    fn toplevel_destroyed(&mut self, surface: ToplevelSurface) {
        if let Some(window) = self.window_for_surface(surface.wl_surface()) {
            self.unmanage_window(&window);
        }
    }

    fn title_changed(&mut self, surface: ToplevelSurface) {
        if let Some(window) = self.window_for_surface(surface.wl_surface()) {
            self.refresh_window_info(&window);
        }
    }

    fn app_id_changed(&mut self, surface: ToplevelSurface) {
        if let Some(window) = self.window_for_surface(surface.wl_surface()) {
            self.refresh_window_info(&window);
        }
    }

    fn minimize_request(&mut self, surface: ToplevelSurface) {
        if let Some(id) = self
            .window_for_surface(surface.wl_surface())
            .and_then(|w| self.window_id(&w))
        {
            self.wm.minimize(Some(id));
            self.mark_layout();
        }
    }

    fn new_popup(&mut self, surface: PopupSurface, _positioner: PositionerState) {
        // Do not send a configure here, the initial configure
        // of a xdg_surface has to be sent during the commit if
        // the surface is not already configured

        self.unconstrain_popup(&surface);

        if let Err(err) = self.popups.track_popup(PopupKind::from(surface)) {
            warn!("Failed to track popup: {}", err);
        }
    }

    fn reposition_request(
        &mut self,
        surface: PopupSurface,
        positioner: PositionerState,
        token: u32,
    ) {
        surface.with_pending_state(|state| {
            let geometry = positioner.get_geometry();
            state.geometry = geometry;
            state.positioner = positioner;
        });
        self.unconstrain_popup(&surface);
        surface.send_repositioned(token);
    }

    fn move_request(&mut self, surface: ToplevelSurface, seat: wl_seat::WlSeat, serial: Serial) {
        let seat: Seat<EdexState<BackendData>> = Seat::from_resource(&seat).unwrap();
        self.move_request_xdg(&surface, &seat, serial)
    }

    fn resize_request(
        &mut self,
        surface: ToplevelSurface,
        seat: wl_seat::WlSeat,
        serial: Serial,
        edges: xdg_toplevel::ResizeEdge,
    ) {
        let seat: Seat<EdexState<BackendData>> = Seat::from_resource(&seat).unwrap();
        if !self.is_floating_surface(surface.wl_surface()) {
            return;
        }

        if let Some(touch) = seat.get_touch() {
            if touch.has_grab(serial) {
                let start_data = touch.grab_start_data().unwrap();
                tracing::info!(?start_data);

                // If the client disconnects after requesting a move
                // we can just ignore the request
                let Some(window) = self.window_for_surface(surface.wl_surface()) else {
                    tracing::info!("no window");
                    return;
                };

                // If the focus was for a different surface, ignore the request.
                if start_data.focus.is_none()
                    || !start_data
                        .focus
                        .as_ref()
                        .unwrap()
                        .0
                        .same_client_as(&surface.wl_surface().id())
                {
                    tracing::info!("different surface");
                    return;
                }
                let geometry = window.geometry();
                let loc = self.space.element_location(&window).unwrap();
                let (initial_window_location, initial_window_size) = (loc, geometry.size);

                with_states(surface.wl_surface(), move |states| {
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

                let grab = TouchResizeSurfaceGrab {
                    start_data,
                    window,
                    edges: edges.into(),
                    initial_window_location,
                    initial_window_size,
                    last_window_size: initial_window_size,
                };

                touch.set_grab(self, grab, serial);
                return;
            }
        }

        let pointer = seat.get_pointer().unwrap();

        // Check that this surface has a click grab.
        if !pointer.has_grab(serial) {
            return;
        }

        let start_data = pointer.grab_start_data().unwrap();

        let window = self.window_for_surface(surface.wl_surface()).unwrap();

        // If the focus was for a different surface, ignore the request.
        if start_data.focus.is_none()
            || !start_data
                .focus
                .as_ref()
                .unwrap()
                .0
                .same_client_as(&surface.wl_surface().id())
        {
            return;
        }

        let geometry = window.geometry();
        let loc = self.space.element_location(&window).unwrap();
        let (initial_window_location, initial_window_size) = (loc, geometry.size);

        with_states(surface.wl_surface(), move |states| {
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
            window,
            edges: edges.into(),
            initial_window_location,
            initial_window_size,
            last_window_size: initial_window_size,
        };

        pointer.set_grab(self, grab, serial, Focus::Clear);
    }

    fn ack_configure(&mut self, surface: WlSurface, configure: Configure) {
        if let Configure::Toplevel(configure) = configure {
            if let Some(serial) = with_states(&surface, |states| {
                if let Some(data) = states.data_map.get::<RefCell<SurfaceData>>() {
                    if let ResizeState::WaitingForFinalAck(_, serial) = data.borrow().resize_state {
                        return Some(serial);
                    }
                }

                None
            }) {
                // When the resize grab is released the surface
                // resize state will be set to WaitingForFinalAck
                // and the client will receive a configure request
                // without the resize state to inform the client
                // resizing has finished. Here we will wait for
                // the client to acknowledge the end of the
                // resizing. To check if the surface was resizing
                // before sending the configure we need to use
                // the current state as the received acknowledge
                // will no longer have the resize state set
                let is_resizing = with_states(&surface, |states| {
                    states
                        .data_map
                        .get::<XdgToplevelSurfaceData>()
                        .unwrap()
                        .lock()
                        .unwrap()
                        .current
                        .states
                        .contains(xdg_toplevel::State::Resizing)
                });

                if configure.serial >= serial && is_resizing {
                    with_states(&surface, |states| {
                        let mut data = states
                            .data_map
                            .get::<RefCell<SurfaceData>>()
                            .unwrap()
                            .borrow_mut();
                        if let ResizeState::WaitingForFinalAck(resize_data, _) = data.resize_state {
                            data.resize_state = ResizeState::WaitingForCommit(resize_data);
                        } else {
                            unreachable!()
                        }
                    });
                }
            }
        }
    }

    fn fullscreen_request(
        &mut self,
        surface: ToplevelSurface,
        _output: Option<wl_output::WlOutput>,
    ) {
        self.mode_request(&surface, WindowMode::Fullscreen, true);
    }

    fn unfullscreen_request(&mut self, surface: ToplevelSurface) {
        self.mode_request(&surface, WindowMode::Fullscreen, false);
    }

    fn maximize_request(&mut self, surface: ToplevelSurface) {
        self.mode_request(&surface, WindowMode::Maximized, true);
    }

    fn unmaximize_request(&mut self, surface: ToplevelSurface) {
        self.mode_request(&surface, WindowMode::Maximized, false);
    }

    fn grab(&mut self, surface: PopupSurface, seat: wl_seat::WlSeat, serial: Serial) {
        let seat: Seat<EdexState<BackendData>> = Seat::from_resource(&seat).unwrap();
        let kind = PopupKind::Xdg(surface);
        if let Some(root) = find_popup_root_surface(&kind).ok().and_then(|root| {
            self.space
                .elements()
                .find(|w| w.wl_surface().map(|s| *s == root).unwrap_or(false))
                .cloned()
                .map(KeyboardFocusTarget::from)
                .or_else(|| {
                    self.space
                        .outputs()
                        .find_map(|o| {
                            let map = layer_map_for_output(o);
                            map.layer_for_surface(&root, WindowSurfaceType::TOPLEVEL)
                                .cloned()
                        })
                        .map(KeyboardFocusTarget::LayerSurface)
                })
        }) {
            let ret = self.popups.grab_popup(root, kind, &seat, serial);

            if let Ok(mut grab) = ret {
                if let Some(keyboard) = seat.get_keyboard() {
                    if keyboard.is_grabbed()
                        && !(keyboard.has_grab(serial)
                            || keyboard.has_grab(grab.previous_serial().unwrap_or(serial)))
                    {
                        grab.ungrab(PopupUngrabStrategy::All);
                        return;
                    }
                    keyboard.set_focus(self, grab.current_grab(), serial);
                    keyboard.set_grab(self, PopupKeyboardGrab::new(&grab), serial);
                }
                if let Some(pointer) = seat.get_pointer() {
                    if pointer.is_grabbed()
                        && !(pointer.has_grab(serial)
                            || pointer
                                .has_grab(grab.previous_serial().unwrap_or_else(|| grab.serial())))
                    {
                        grab.ungrab(PopupUngrabStrategy::All);
                        return;
                    }
                    pointer.set_grab(self, PopupPointerGrab::new(&grab), serial, Focus::Keep);
                }
            }
        }
    }
}

impl<BackendData: Backend> EdexState<BackendData> {
    /// A client's (un)maximize or (un)fullscreen request. The protocol wants a configure in
    /// reply whether or not anything changed.
    fn mode_request(&mut self, surface: &ToplevelSurface, mode: WindowMode, on: bool) {
        if let Some(id) = self
            .window_for_surface(surface.wl_surface())
            .and_then(|w| self.window_id(&w))
        {
            self.wm.request_mode(id, mode, on);
            self.mark_layout();
            self.flush();
        }
        if surface.is_initial_configure_sent() {
            surface.send_configure();
        }
    }

    /// Floating windows can be moved and resized interactively; tiled ones stay in their tile.
    pub fn is_floating_surface(&self, surface: &WlSurface) -> bool {
        self.window_for_surface(surface)
            .and_then(|w| self.window_id(&w))
            .is_some_and(|id| self.wm.is_floating(id))
    }

    /// After the first commit with content: fixed-size windows (min == max) float at their
    /// own size.
    pub fn first_commit(&mut self, window: &WindowElement) {
        struct Seen;
        if window.user_data().get::<Seen>().is_some() {
            return;
        }
        let Some(toplevel) = window.0.toplevel() else {
            return;
        };
        let (min, max) = with_states(toplevel.wl_surface(), |states| {
            let mut cached = states
                .cached_state
                .get::<smithay::wayland::shell::xdg::SurfaceCachedState>();
            let c = cached.current();
            (c.min_size, c.max_size)
        });
        let geo = window.geometry();
        if geo.size.is_empty() {
            return;
        }
        window.user_data().insert_if_missing(|| Seen);
        if min.w > 0 && min == max {
            if let Some(id) = self.window_id(window) {
                if !self.wm.is_floating(id) {
                    self.wm
                        .toggle_float(Some(id), Some(crate::manage::to_rect(geo)));
                    self.mark_layout();
                }
            }
        }
    }

    pub fn move_request_xdg(
        &mut self,
        surface: &ToplevelSurface,
        seat: &Seat<Self>,
        serial: Serial,
    ) {
        if !self.is_floating_surface(surface.wl_surface()) {
            return;
        }
        if let Some(touch) = seat.get_touch() {
            if touch.has_grab(serial) {
                let start_data = touch.grab_start_data().unwrap();

                // If the client disconnects after requesting a move
                // we can just ignore the request
                let Some(window) = self.window_for_surface(surface.wl_surface()) else {
                    return;
                };

                // If the focus was for a different surface, ignore the request.
                if start_data.focus.is_none()
                    || !start_data
                        .focus
                        .as_ref()
                        .unwrap()
                        .0
                        .same_client_as(&surface.wl_surface().id())
                {
                    return;
                }

                let mut initial_window_location = self.space.element_location(&window).unwrap();

                // If surface is maximized then unmaximize it
                let current_state = surface.current_state();
                if current_state
                    .states
                    .contains(xdg_toplevel::State::Maximized)
                {
                    surface.with_pending_state(|state| {
                        state.states.unset(xdg_toplevel::State::Maximized);
                        state.size = None;
                    });

                    surface.send_configure();

                    // NOTE: In real compositor mouse location should be mapped to a new window size
                    // For example, you could:
                    // 1) transform mouse pointer position from compositor space to window space (location relative)
                    // 2) divide the x coordinate by width of the window to get the percentage
                    //   - 0.0 would be on the far left of the window
                    //   - 0.5 would be in middle of the window
                    //   - 1.0 would be on the far right of the window
                    // 3) multiply the percentage by new window width
                    // 4) by doing that, drag will look a lot more natural
                    //
                    // but for anvil needs setting location to pointer location is fine
                    initial_window_location = start_data.location.to_i32_round();
                }

                let grab = TouchMoveSurfaceGrab {
                    start_data,
                    window,
                    initial_window_location,
                };

                touch.set_grab(self, grab, serial);
                return;
            }
        }

        let pointer = seat.get_pointer().unwrap();

        // Check that this surface has a click grab.
        if !pointer.has_grab(serial) {
            return;
        }

        let start_data = pointer.grab_start_data().unwrap();

        // If the client disconnects after requesting a move
        // we can just ignore the request
        let Some(window) = self.window_for_surface(surface.wl_surface()) else {
            return;
        };

        // If the focus was for a different surface, ignore the request.
        if start_data.focus.is_none()
            || !start_data
                .focus
                .as_ref()
                .unwrap()
                .0
                .same_client_as(&surface.wl_surface().id())
        {
            return;
        }

        let mut initial_window_location = self.space.element_location(&window).unwrap();

        // If surface is maximized then unmaximize it
        let current_state = surface.current_state();
        if current_state
            .states
            .contains(xdg_toplevel::State::Maximized)
        {
            surface.with_pending_state(|state| {
                state.states.unset(xdg_toplevel::State::Maximized);
                state.size = None;
            });

            surface.send_configure();

            // NOTE: In real compositor mouse location should be mapped to a new window size
            // For example, you could:
            // 1) transform mouse pointer position from compositor space to window space (location relative)
            // 2) divide the x coordinate by width of the window to get the percentage
            //   - 0.0 would be on the far left of the window
            //   - 0.5 would be in middle of the window
            //   - 1.0 would be on the far right of the window
            // 3) multiply the percentage by new window width
            // 4) by doing that, drag will look a lot more natural
            //
            // but for anvil needs setting location to pointer location is fine
            let pos = pointer.current_location();
            initial_window_location = (pos.x as i32, pos.y as i32).into();
        }

        let grab = PointerMoveSurfaceGrab {
            start_data,
            window,
            initial_window_location,
        };

        pointer.set_grab(self, grab, serial, Focus::Clear);
    }

    fn unconstrain_popup(&self, popup: &PopupSurface) {
        let Ok(root) = find_popup_root_surface(&PopupKind::Xdg(popup.clone())) else {
            return;
        };
        let Some(window) = self.window_for_surface(&root) else {
            return;
        };

        let mut outputs_for_window = self.space.outputs_for_element(&window);
        if outputs_for_window.is_empty() {
            return;
        }

        // Get a union of all outputs' geometries.
        let mut outputs_geo = self
            .space
            .output_geometry(&outputs_for_window.pop().unwrap())
            .unwrap();
        for output in outputs_for_window {
            outputs_geo = outputs_geo.merge(self.space.output_geometry(&output).unwrap());
        }

        let window_geo = self.space.element_geometry(&window).unwrap();

        // The target geometry for the positioner should be relative to its parent's geometry, so
        // we will compute that here.
        let mut target = outputs_geo;
        target.loc -= get_popup_toplevel_coords(&PopupKind::Xdg(popup.clone()));
        target.loc -= window_geo.loc;

        popup.with_pending_state(|state| {
            state.geometry = state.positioner.get_unconstrained_geometry(target);
        });
    }
}

/// Should be called on `WlSurface::commit` of xdg toplevel
fn handle_toplevel_commit(space: &mut Space<WindowElement>, surface: &WlSurface) -> Option<()> {
    let window = space
        .elements()
        .find(|w| w.wl_surface().as_deref() == Some(surface))
        .cloned()?;

    let mut window_loc = space.element_location(&window)?;
    let geometry = window.geometry();

    let new_loc: Point<Option<i32>, Logical> =
        with_states(window.wl_surface().as_deref()?, |states| {
            let data = states.data_map.get::<RefCell<SurfaceData>>()?.borrow_mut();

            if let ResizeState::Resizing(resize_data) = data.resize_state {
                let edges = resize_data.edges;
                let loc = resize_data.initial_window_location;
                let size = resize_data.initial_window_size;

                // If the window is being resized by top or left, its location must be adjusted
                // accordingly.
                edges.intersects(ResizeEdge::TOP_LEFT).then(|| {
                    let new_x = edges
                        .intersects(ResizeEdge::LEFT)
                        .then_some(loc.x + (size.w - geometry.size.w));

                    let new_y = edges
                        .intersects(ResizeEdge::TOP)
                        .then_some(loc.y + (size.h - geometry.size.h));

                    (new_x, new_y).into()
                })
            } else {
                None
            }
        })?;

    if let Some(new_x) = new_loc.x {
        window_loc.x = new_x;
    }
    if let Some(new_y) = new_loc.y {
        window_loc.y = new_y;
    }

    if new_loc.x.is_some() || new_loc.y.is_some() {
        // If TOP or LEFT side of the window got resized, we have to move it
        space.map_element(window, window_loc, false);
    }

    Some(())
}
