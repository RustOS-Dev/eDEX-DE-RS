use std::convert::TryInto;

use std::time::Duration;

use crate::{
    binds::{self, Action, Mods},
    focus::{KeyboardFocusTarget, PointerFocusTarget},
    shell::FullscreenSurface,
    EdexState,
};
use smithay::desktop::space::SpaceElement;
use smithay::reexports::calloop::timer::{TimeoutAction, Timer};

use crate::udev::UdevData;

use smithay::{
    backend::input::{
        self, Axis, AxisSource, Event, InputBackend, InputEvent, KeyState, KeyboardKeyEvent,
        PointerAxisEvent, PointerButtonEvent,
    },
    desktop::{layer_map_for_output, WindowSurfaceType},
    input::{
        keyboard::{keysyms as xkb, FilterResult, Keysym},
        pointer::{AxisFrame, ButtonEvent, MotionEvent},
    },
    reexports::wayland_server::protocol::wl_pointer,
    utils::{Logical, Point, Serial, SERIAL_COUNTER as SCOUNTER},
    wayland::{
        compositor::with_states,
        input_method::InputMethodSeat,
        keyboard_shortcuts_inhibit::KeyboardShortcutsInhibitorSeat,
        shell::wlr_layer::{KeyboardInteractivity, Layer as WlrLayer, LayerSurfaceCachedState},
    },
};

use smithay::backend::input::AbsolutePositionEvent;

use smithay::output::Output;

use crate::state::Backend;
use smithay::{
    backend::input::{
        Device, DeviceCapability, GestureBeginEvent, GestureEndEvent, GesturePinchUpdateEvent as _,
        GestureSwipeUpdateEvent as _, PointerMotionEvent, ProximityState, TabletToolButtonEvent,
        TabletToolEvent, TabletToolProximityEvent, TabletToolTipEvent, TabletToolTipState,
        TouchEvent,
    },
    input::{
        pointer::{
            GestureHoldBeginEvent, GestureHoldEndEvent, GesturePinchBeginEvent,
            GesturePinchEndEvent, GesturePinchUpdateEvent, GestureSwipeBeginEvent,
            GestureSwipeEndEvent, GestureSwipeUpdateEvent, RelativeMotionEvent,
        },
        touch::{DownEvent, UpEvent},
    },
    reexports::wayland_server::DisplayHandle,
    wayland::{
        pointer_constraints::{with_pointer_constraint, PointerConstraint},
        seat::WaylandFocus,
        tablet_manager::{TabletDescriptor, TabletSeatTrait},
    },
};

impl<BackendData: Backend> EdexState<BackendData> {
    /// Key events: compositor bindings first, then the focused client.
    fn on_keyboard<B: InputBackend>(&mut self, evt: B::KeyboardKeyEvent) {
        let keycode = evt.key_code();
        let state = evt.state();
        let serial = SCOUNTER.next_serial();
        let time = Event::time_msec(&evt);
        let keyboard = self.seat.get_keyboard().unwrap();
        let locked = self.lock.locked;

        // An exclusive overlay (launcher, settings) keeps the keyboard while it is open.
        if !locked {
            for layer in self.layer_shell_state.layer_surfaces().rev() {
                let data = with_states(layer.wl_surface(), |states| {
                    *states
                        .cached_state
                        .get::<LayerSurfaceCachedState>()
                        .current()
                });
                if data.keyboard_interactivity == KeyboardInteractivity::Exclusive
                    && (data.layer == WlrLayer::Top || data.layer == WlrLayer::Overlay)
                {
                    let surface = self.space.outputs().find_map(|o| {
                        let map = layer_map_for_output(o);
                        let cloned = map.layers().find(|l| l.layer_surface() == &layer).cloned();
                        cloned
                    });
                    if let Some(surface) = surface {
                        let target = KeyboardFocusTarget::LayerSurface(surface);
                        if keyboard.current_focus().as_ref() != Some(&target) {
                            keyboard.set_focus(self, Some(target), serial);
                        }
                        break;
                    }
                }
            }
        }

        let inhibited = !locked
            && keyboard
                .current_focus()
                .and_then(|f| f.wl_surface().map(|s| s.into_owned()))
                .and_then(|surface| self.seat.keyboard_shortcuts_inhibitor_for_surface(&surface))
                .map(|inhibitor| inhibitor.is_active())
                .unwrap_or(false);

        let binds = self.config.binds.clone();
        let mut suppressed = std::mem::take(&mut self.suppressed_keys);
        let mut pending_tap = self.pending_tap.take();
        let mut tapped: Option<Action> = None;
        let mut repeat_bind = false;

        let action = keyboard.input(
            self,
            keycode,
            state,
            serial,
            time,
            |_, modifiers, handle| {
                let raw: Vec<Keysym> = handle.raw_syms().to_vec();
                let sym = raw.first().copied().unwrap_or(handle.modified_sym());
                match state {
                    KeyState::Pressed => {
                        let lone_modifier = is_modifier(sym);
                        pending_tap = lone_modifier.then_some(sym);
                        if inhibited {
                            return FilterResult::Forward;
                        }
                        match binds::lookup(&binds, Mods::from(modifiers), &raw) {
                            Some(b) if !locked || b.locked => {
                                suppressed.push(sym);
                                repeat_bind = b.repeat;
                                FilterResult::Intercept(b.action.clone())
                            }
                            _ => FilterResult::Forward,
                        }
                    }
                    KeyState::Released => {
                        if pending_tap.take() == Some(sym) && !inhibited && !locked {
                            tapped = binds::lookup_tap(&binds, sym).map(|b| b.action.clone());
                        }
                        if let Some(pos) = suppressed.iter().position(|k| *k == sym) {
                            suppressed.remove(pos);
                            FilterResult::Intercept(Action::None)
                        } else {
                            FilterResult::Forward
                        }
                    }
                }
            },
        );

        self.suppressed_keys = suppressed;
        self.pending_tap = pending_tap;
        if state == KeyState::Released {
            self.stop_bind_repeat();
        }
        if let Some(action) = action {
            if action != Action::None {
                if repeat_bind {
                    self.start_bind_repeat(action.clone());
                }
                self.run_action(action);
            }
        }
        if let Some(action) = tapped {
            self.run_action(action);
        }
    }

    /// Volume and brightness keys repeat while held, at the keyboard's repeat rate.
    fn start_bind_repeat(&mut self, action: Action) {
        self.stop_bind_repeat();
        let delay = Duration::from_millis(self.config.xkb.repeat_delay.max(100) as u64);
        let every = Duration::from_millis(1000 / self.config.xkb.repeat_rate.max(1) as u64);
        let token = self
            .handle
            .insert_source(Timer::from_duration(delay), move |_, _, state| {
                state.run_action(action.clone());
                TimeoutAction::ToDuration(every)
            })
            .ok();
        self.bind_repeat = token;
    }

    fn stop_bind_repeat(&mut self) {
        if let Some(token) = self.bind_repeat.take() {
            self.handle.remove(token);
        }
    }

    fn on_pointer_button<B: InputBackend>(&mut self, evt: B::PointerButtonEvent) {
        let serial = SCOUNTER.next_serial();
        let button = evt.button_code();

        let state = wl_pointer::ButtonState::from(evt.state());

        if wl_pointer::ButtonState::Pressed == state {
            if !self.lock.locked && self.mouse_bind(button, serial) {
                return;
            }
            self.update_keyboard_focus(self.pointer.current_location(), serial);
        };
        let pointer = self.pointer.clone();
        pointer.button(
            self,
            &ButtonEvent {
                button,
                state: state.try_into().unwrap(),
                serial,
                time: evt.time_msec(),
            },
        );
        pointer.frame(self);
    }

    /// SUPER + left button drags a window (floating it), SUPER + right button resizes it.
    fn mouse_bind(&mut self, button: u32, serial: Serial) -> bool {
        const BTN_LEFT: u32 = 0x110;
        const BTN_RIGHT: u32 = 0x111;
        let keyboard = self.seat.get_keyboard().unwrap();
        if !keyboard.modifier_state().logo || !(button == BTN_LEFT || button == BTN_RIGHT) {
            return false;
        }
        let location = self.pointer.current_location();
        let Some((window, _)) = self
            .space
            .element_under(location)
            .map(|(w, p)| (w.clone(), p))
        else {
            return false;
        };
        let Some(id) = self.window_id(&window) else {
            return false;
        };
        if !self.wm.is_floating(id) {
            let current = self
                .space
                .element_geometry(&window)
                .map(crate::manage::to_rect);
            self.wm.toggle_float(Some(id), current);
        }
        self.wm.focus_window(id);
        self.mark_layout();
        self.flush();
        let Some(initial_window_location) = self.space.element_location(&window) else {
            return false;
        };
        let start_data = smithay::input::pointer::GrabStartData {
            focus: None,
            button,
            location,
        };
        let pointer = self.pointer.clone();
        if button == BTN_LEFT {
            let grab = crate::shell::PointerMoveSurfaceGrab {
                start_data,
                window,
                initial_window_location,
            };
            pointer.set_grab(self, grab, serial, smithay::input::pointer::Focus::Clear);
        } else {
            let geometry = window.geometry();
            let edges = crate::shell::ResizeEdge::BOTTOM_RIGHT;
            if let Some(surface) = window.wl_surface() {
                with_states(&surface, |states| {
                    if let Some(data) = states
                        .data_map
                        .get::<std::cell::RefCell<crate::shell::SurfaceData>>()
                    {
                        data.borrow_mut().resize_state =
                            crate::shell::ResizeState::Resizing(crate::shell::ResizeData {
                                edges,
                                initial_window_location,
                                initial_window_size: geometry.size,
                            });
                    }
                });
            }
            let grab = crate::shell::PointerResizeSurfaceGrab {
                start_data,
                window,
                edges,
                initial_window_location,
                initial_window_size: geometry.size,
                last_window_size: geometry.size,
            };
            pointer.set_grab(self, grab, serial, smithay::input::pointer::Focus::Clear);
        }
        true
    }

    fn update_keyboard_focus(&mut self, location: Point<f64, Logical>, serial: Serial) {
        let (x, y) = crate::manage::point_i32(location);
        self.wm.focus_output_at(x, y);
        if self.lock.locked {
            return;
        }
        let keyboard = self.seat.get_keyboard().unwrap();
        let touch = self.seat.get_touch();
        let input_method = self.seat.input_method();
        // change the keyboard focus unless the pointer or keyboard is grabbed
        // We test for any matching surface type here but always use the root
        // (in case of a window the toplevel) surface for the focus.
        // So for example if a user clicks on a subsurface or popup the toplevel
        // will receive the keyboard focus. Directly assigning the focus to the
        // matching surface leads to issues with clients dismissing popups and
        // subsurface menus (for example firefox-wayland).
        // see here for a discussion about that issue:
        // https://gitlab.freedesktop.org/wayland/wayland/-/issues/294
        if !self.pointer.is_grabbed()
            && (!keyboard.is_grabbed() || input_method.keyboard_grabbed())
            && !touch.map(|touch| touch.is_grabbed()).unwrap_or(false)
        {
            let output = self.space.output_under(location).next().cloned();
            if let Some(output) = output.as_ref() {
                let output_geo = self.space.output_geometry(output).unwrap();
                if let Some(window) = output
                    .user_data()
                    .get::<FullscreenSurface>()
                    .and_then(|f| f.get())
                {
                    if let Some((_, _)) = window
                        .surface_under(location - output_geo.loc.to_f64(), WindowSurfaceType::ALL)
                    {
                        if let Some(surface) = window.0.x11_surface() {
                            self.xwm.as_mut().unwrap().raise_window(surface).unwrap();
                        }
                        keyboard.set_focus(self, Some(window.into()), serial);
                        return;
                    }
                }

                let layers = layer_map_for_output(output);
                if let Some(layer) = layers
                    .layer_under(WlrLayer::Overlay, location - output_geo.loc.to_f64())
                    .or_else(|| {
                        layers.layer_under(WlrLayer::Top, location - output_geo.loc.to_f64())
                    })
                {
                    if layer.can_receive_keyboard_focus() {
                        if let Some((_, _)) = layer.surface_under(
                            location
                                - output_geo.loc.to_f64()
                                - layers.layer_geometry(layer).unwrap().loc.to_f64(),
                            WindowSurfaceType::ALL,
                        ) {
                            keyboard.set_focus(self, Some(layer.clone().into()), serial);
                            return;
                        }
                    }
                }
            }

            if let Some((window, _)) = self
                .space
                .element_under(location)
                .map(|(w, p)| (w.clone(), p))
            {
                self.space.raise_element(&window, true);
                if let Some(surface) = window.0.x11_surface() {
                    self.xwm.as_mut().unwrap().raise_window(surface).unwrap();
                }
                keyboard.set_focus(self, Some(window.into()), serial);
                return;
            }

            if let Some(output) = output.as_ref() {
                let output_geo = self.space.output_geometry(output).unwrap();
                let layers = layer_map_for_output(output);
                if let Some(layer) = layers
                    .layer_under(WlrLayer::Bottom, location - output_geo.loc.to_f64())
                    .or_else(|| {
                        layers.layer_under(WlrLayer::Background, location - output_geo.loc.to_f64())
                    })
                {
                    if layer.can_receive_keyboard_focus() {
                        if let Some((_, _)) = layer.surface_under(
                            location
                                - output_geo.loc.to_f64()
                                - layers.layer_geometry(layer).unwrap().loc.to_f64(),
                            WindowSurfaceType::ALL,
                        ) {
                            keyboard.set_focus(self, Some(layer.clone().into()), serial);
                        }
                    }
                }
            };
        }
    }

    pub fn surface_under(
        &self,
        pos: Point<f64, Logical>,
    ) -> Option<(PointerFocusTarget, Point<f64, Logical>)> {
        let output = self.space.outputs().find(|o| {
            let geometry = self.space.output_geometry(o).unwrap();
            geometry.contains(pos.to_i32_round())
        })?;
        let output_geo = self.space.output_geometry(output).unwrap();
        if self.lock.locked {
            let surface = self.lock_surface_for(output)?;
            return Some((PointerFocusTarget::from(surface), output_geo.loc.to_f64()));
        }
        let layers = layer_map_for_output(output);

        let mut under = None;
        if let Some((surface, loc)) = output
            .user_data()
            .get::<FullscreenSurface>()
            .and_then(|f| f.get())
            .and_then(|w| w.surface_under(pos - output_geo.loc.to_f64(), WindowSurfaceType::ALL))
        {
            under = Some((surface, loc + output_geo.loc));
        } else if let Some(focus) = layers
            .layer_under(WlrLayer::Overlay, pos - output_geo.loc.to_f64())
            .or_else(|| layers.layer_under(WlrLayer::Top, pos - output_geo.loc.to_f64()))
            .and_then(|layer| {
                let layer_loc = layers.layer_geometry(layer).unwrap().loc;
                layer
                    .surface_under(
                        pos - output_geo.loc.to_f64() - layer_loc.to_f64(),
                        WindowSurfaceType::ALL,
                    )
                    .map(|(surface, loc)| {
                        (
                            PointerFocusTarget::from(surface),
                            loc + layer_loc + output_geo.loc,
                        )
                    })
            })
        {
            under = Some(focus)
        } else if let Some(focus) = self.space.element_under(pos).and_then(|(window, loc)| {
            window
                .surface_under(pos - loc.to_f64(), WindowSurfaceType::ALL)
                .map(|(surface, surf_loc)| (surface, surf_loc + loc))
        }) {
            under = Some(focus);
        } else if let Some(focus) = layers
            .layer_under(WlrLayer::Bottom, pos - output_geo.loc.to_f64())
            .or_else(|| layers.layer_under(WlrLayer::Background, pos - output_geo.loc.to_f64()))
            .and_then(|layer| {
                let layer_loc = layers.layer_geometry(layer).unwrap().loc;
                layer
                    .surface_under(
                        pos - output_geo.loc.to_f64() - layer_loc.to_f64(),
                        WindowSurfaceType::ALL,
                    )
                    .map(|(surface, loc)| {
                        (
                            PointerFocusTarget::from(surface),
                            loc + layer_loc + output_geo.loc,
                        )
                    })
            })
        {
            under = Some(focus)
        };
        under.map(|(s, l)| (s, l.to_f64()))
    }

    fn on_pointer_axis<B: InputBackend>(&mut self, evt: B::PointerAxisEvent) {
        let logo = self.seat.get_keyboard().unwrap().modifier_state().logo;
        if logo && !self.lock.locked {
            let v = evt
                .amount_v120(input::Axis::Vertical)
                .or_else(|| evt.amount(input::Axis::Vertical))
                .unwrap_or(0.0);
            if v != 0.0 {
                let target = if v > 0.0 {
                    comp_proto::WorkspaceTarget::Next
                } else {
                    comp_proto::WorkspaceTarget::Prev
                };
                self.run_action(Action::Workspace(target));
            }
            return;
        }
        let horizontal_amount = evt.amount(input::Axis::Horizontal).unwrap_or_else(|| {
            evt.amount_v120(input::Axis::Horizontal).unwrap_or(0.0) * 15.0 / 120.
        });
        let vertical_amount = evt
            .amount(input::Axis::Vertical)
            .unwrap_or_else(|| evt.amount_v120(input::Axis::Vertical).unwrap_or(0.0) * 15.0 / 120.);
        let horizontal_amount_discrete = evt.amount_v120(input::Axis::Horizontal);
        let vertical_amount_discrete = evt.amount_v120(input::Axis::Vertical);

        {
            let mut frame = AxisFrame::new(evt.time_msec()).source(evt.source());
            if horizontal_amount != 0.0 {
                frame = frame
                    .relative_direction(Axis::Horizontal, evt.relative_direction(Axis::Horizontal));
                frame = frame.value(Axis::Horizontal, horizontal_amount);
                if let Some(discrete) = horizontal_amount_discrete {
                    frame = frame.v120(Axis::Horizontal, discrete as i32);
                }
            }
            if vertical_amount != 0.0 {
                frame = frame
                    .relative_direction(Axis::Vertical, evt.relative_direction(Axis::Vertical));
                frame = frame.value(Axis::Vertical, vertical_amount);
                if let Some(discrete) = vertical_amount_discrete {
                    frame = frame.v120(Axis::Vertical, discrete as i32);
                }
            }
            if evt.source() == AxisSource::Finger {
                if evt.amount(Axis::Horizontal) == Some(0.0) {
                    frame = frame.stop(Axis::Horizontal);
                }
                if evt.amount(Axis::Vertical) == Some(0.0) {
                    frame = frame.stop(Axis::Vertical);
                }
            }
            let pointer = self.pointer.clone();
            pointer.axis(self, frame);
            pointer.frame(self);
        }
    }
}

impl<BackendData: Backend> EdexState<BackendData> {
    pub fn process_input_event_windowed<B: InputBackend>(
        &mut self,
        event: InputEvent<B>,
        output_name: &str,
    ) {
        self.on_activity();
        match event {
            InputEvent::Keyboard { event } => self.on_keyboard::<B>(event),
            InputEvent::PointerMotionAbsolute { event } => {
                let Some(output) = self
                    .space
                    .outputs()
                    .find(|o| o.name() == output_name)
                    .cloned()
                else {
                    return;
                };
                self.on_pointer_move_absolute_windowed::<B>(event, &output)
            }
            InputEvent::PointerButton { event } => self.on_pointer_button::<B>(event),
            InputEvent::PointerAxis { event } => self.on_pointer_axis::<B>(event),
            _ => (),
        }
    }

    fn on_pointer_move_absolute_windowed<B: InputBackend>(
        &mut self,
        evt: B::PointerMotionAbsoluteEvent,
        output: &Output,
    ) {
        let output_geo = self.space.output_geometry(output).unwrap();

        let pos = evt.position_transformed(output_geo.size) + output_geo.loc.to_f64();
        let serial = SCOUNTER.next_serial();

        let pointer = self.pointer.clone();
        let under = self.surface_under(pos);
        pointer.motion(
            self,
            under,
            &MotionEvent {
                location: pos,
                serial,
                time: evt.time_msec(),
            },
        );
        pointer.frame(self);
    }

    pub fn release_all_keys(&mut self) {
        let keyboard = self.seat.get_keyboard().unwrap();
        for keycode in keyboard.pressed_keys() {
            keyboard.input(
                self,
                keycode,
                KeyState::Released,
                SCOUNTER.next_serial(),
                0,
                |_, _, _| FilterResult::Forward::<bool>,
            );
        }
    }
}

impl EdexState<UdevData> {
    pub fn process_input_event<B: InputBackend>(
        &mut self,
        dh: &DisplayHandle,
        event: InputEvent<B>,
    ) {
        if !matches!(
            event,
            InputEvent::DeviceAdded { .. } | InputEvent::DeviceRemoved { .. }
        ) {
            self.on_activity();
        }
        match event {
            InputEvent::Keyboard { event, .. } => self.on_keyboard::<B>(event),
            InputEvent::SwitchToggle { event } => {
                use smithay::backend::input::{Switch, SwitchState, SwitchToggleEvent};
                if event.switch() == Some(Switch::Lid) {
                    self.on_lid(event.state() == SwitchState::On);
                }
            }
            InputEvent::PointerMotion { event, .. } => self.on_pointer_move::<B>(dh, event),
            InputEvent::PointerMotionAbsolute { event, .. } => {
                self.on_pointer_move_absolute::<B>(dh, event)
            }
            InputEvent::PointerButton { event, .. } => self.on_pointer_button::<B>(event),
            InputEvent::PointerAxis { event, .. } => self.on_pointer_axis::<B>(event),
            InputEvent::TabletToolAxis { event, .. } => self.on_tablet_tool_axis::<B>(event),
            InputEvent::TabletToolProximity { event, .. } => {
                self.on_tablet_tool_proximity::<B>(dh, event)
            }
            InputEvent::TabletToolTip { event, .. } => self.on_tablet_tool_tip::<B>(event),
            InputEvent::TabletToolButton { event, .. } => self.on_tablet_button::<B>(event),
            InputEvent::GestureSwipeBegin { event, .. } => self.on_gesture_swipe_begin::<B>(event),
            InputEvent::GestureSwipeUpdate { event, .. } => {
                self.on_gesture_swipe_update::<B>(event)
            }
            InputEvent::GestureSwipeEnd { event, .. } => self.on_gesture_swipe_end::<B>(event),
            InputEvent::GesturePinchBegin { event, .. } => self.on_gesture_pinch_begin::<B>(event),
            InputEvent::GesturePinchUpdate { event, .. } => {
                self.on_gesture_pinch_update::<B>(event)
            }
            InputEvent::GesturePinchEnd { event, .. } => self.on_gesture_pinch_end::<B>(event),
            InputEvent::GestureHoldBegin { event, .. } => self.on_gesture_hold_begin::<B>(event),
            InputEvent::GestureHoldEnd { event, .. } => self.on_gesture_hold_end::<B>(event),

            InputEvent::TouchDown { event } => self.on_touch_down::<B>(event),
            InputEvent::TouchUp { event } => self.on_touch_up::<B>(event),
            InputEvent::TouchMotion { event } => self.on_touch_motion::<B>(event),
            InputEvent::TouchFrame { event } => self.on_touch_frame::<B>(event),
            InputEvent::TouchCancel { event } => self.on_touch_cancel::<B>(event),

            InputEvent::DeviceAdded { device } => {
                if device.has_capability(DeviceCapability::TabletTool) {
                    self.seat
                        .tablet_seat()
                        .add_tablet::<Self>(dh, &TabletDescriptor::from(&device));
                }
                if device.has_capability(DeviceCapability::Touch) && self.seat.get_touch().is_none()
                {
                    self.seat.add_touch();
                }
            }
            InputEvent::DeviceRemoved { device }
                if device.has_capability(DeviceCapability::TabletTool) =>
            {
                let tablet_seat = self.seat.tablet_seat();

                tablet_seat.remove_tablet(&TabletDescriptor::from(&device));

                // If there are no tablets in seat we can remove all tools
                if tablet_seat.count_tablets() == 0 {
                    tablet_seat.clear_tools();
                }
            }
            _ => {
                // other events are not handled in anvil (yet)
            }
        }
    }

    fn on_pointer_move<B: InputBackend>(
        &mut self,
        _dh: &DisplayHandle,
        evt: B::PointerMotionEvent,
    ) {
        let mut pointer_location = self.pointer.current_location();
        let serial = SCOUNTER.next_serial();

        let pointer = self.pointer.clone();
        let under = self.surface_under(pointer_location);

        let mut pointer_locked = false;
        let mut pointer_confined = false;
        let mut confine_region = None;
        if let Some((surface, surface_loc)) = under
            .as_ref()
            .and_then(|(target, l)| Some((target.wl_surface()?, l)))
        {
            with_pointer_constraint(&surface, &pointer, |constraint| match constraint {
                Some(constraint) if constraint.is_active() => {
                    // Constraint does not apply if not within region
                    if !constraint.region().is_none_or(|x| {
                        x.contains((pointer_location - *surface_loc).to_i32_round())
                    }) {
                        return;
                    }
                    match &*constraint {
                        PointerConstraint::Locked(_locked) => {
                            pointer_locked = true;
                        }
                        PointerConstraint::Confined(confine) => {
                            pointer_confined = true;
                            confine_region = confine.region().cloned();
                        }
                    }
                }
                _ => {}
            });
        }

        pointer.relative_motion(
            self,
            under.clone(),
            &RelativeMotionEvent {
                delta: evt.delta(),
                delta_unaccel: evt.delta_unaccel(),
                utime: evt.time(),
            },
        );

        // If pointer is locked, only emit relative motion
        if pointer_locked {
            pointer.frame(self);
            return;
        }

        pointer_location += evt.delta();

        // clamp to screen limits
        // this event is never generated by winit
        pointer_location = self.clamp_coords(pointer_location);

        let new_under = self.surface_under(pointer_location);

        // If confined, don't move pointer if it would go outside surface or region
        if pointer_confined {
            if let Some((surface, surface_loc)) = &under {
                if new_under.as_ref().and_then(|(under, _)| under.wl_surface())
                    != surface.wl_surface()
                {
                    pointer.frame(self);
                    return;
                }
                if let Some(region) = confine_region {
                    if !region.contains((pointer_location - *surface_loc).to_i32_round()) {
                        pointer.frame(self);
                        return;
                    }
                }
            }
        }

        pointer.motion(
            self,
            under,
            &MotionEvent {
                location: pointer_location,
                serial,
                time: evt.time_msec(),
            },
        );
        pointer.frame(self);

        // If pointer is now in a constraint region, activate it
        // TODO Anywhere else pointer is moved needs to do this
        if let Some((under, surface_location)) =
            new_under.and_then(|(target, loc)| Some((target.wl_surface()?.into_owned(), loc)))
        {
            with_pointer_constraint(&under, &pointer, |constraint| match constraint {
                Some(constraint) if !constraint.is_active() => {
                    let point = (pointer_location - surface_location).to_i32_round();
                    if constraint
                        .region()
                        .is_none_or(|region| region.contains(point))
                    {
                        constraint.activate();
                    }
                }
                _ => {}
            });
        }
    }

    fn on_pointer_move_absolute<B: InputBackend>(
        &mut self,
        _dh: &DisplayHandle,
        evt: B::PointerMotionAbsoluteEvent,
    ) {
        let serial = SCOUNTER.next_serial();

        let max_x = self.space.outputs().fold(0, |acc, o| {
            acc + self.space.output_geometry(o).unwrap().size.w
        });

        let max_h_output = self
            .space
            .outputs()
            .max_by_key(|o| self.space.output_geometry(o).unwrap().size.h)
            .unwrap();

        let max_y = self.space.output_geometry(max_h_output).unwrap().size.h;

        let mut pointer_location = (evt.x_transformed(max_x), evt.y_transformed(max_y)).into();

        // clamp to screen limits
        pointer_location = self.clamp_coords(pointer_location);

        let pointer = self.pointer.clone();
        let under = self.surface_under(pointer_location);

        pointer.motion(
            self,
            under,
            &MotionEvent {
                location: pointer_location,
                serial,
                time: evt.time_msec(),
            },
        );
        pointer.frame(self);
    }

    fn on_tablet_tool_axis<B: InputBackend>(&mut self, evt: B::TabletToolAxisEvent) {
        let tablet_seat = self.seat.tablet_seat();

        if let Some(pointer_location) = self.touch_location_transformed(&evt) {
            let pointer = self.pointer.clone();
            let under = self.surface_under(pointer_location);
            let tablet = tablet_seat.get_tablet(&TabletDescriptor::from(&evt.device()));
            let tool = tablet_seat.get_tool(&evt.tool());

            pointer.motion(
                self,
                under.clone(),
                &MotionEvent {
                    location: pointer_location,
                    serial: SCOUNTER.next_serial(),
                    time: self.clock.now().as_millis(),
                },
            );

            if let (Some(tablet), Some(tool)) = (tablet, tool) {
                if evt.pressure_has_changed() {
                    tool.pressure(evt.pressure());
                }
                if evt.distance_has_changed() {
                    tool.distance(evt.distance());
                }
                if evt.tilt_has_changed() {
                    tool.tilt(evt.tilt());
                }
                if evt.slider_has_changed() {
                    tool.slider_position(evt.slider_position());
                }
                if evt.rotation_has_changed() {
                    tool.rotation(evt.rotation());
                }
                if evt.wheel_has_changed() {
                    tool.wheel(evt.wheel_delta(), evt.wheel_delta_discrete());
                }

                tool.motion(
                    pointer_location,
                    under.and_then(|(f, loc)| f.wl_surface().map(|s| (s.into_owned(), loc))),
                    &tablet,
                    SCOUNTER.next_serial(),
                    evt.time_msec(),
                );
            }

            pointer.frame(self);
        }
    }

    fn on_tablet_tool_proximity<B: InputBackend>(
        &mut self,
        dh: &DisplayHandle,
        evt: B::TabletToolProximityEvent,
    ) {
        let tablet_seat = self.seat.tablet_seat();

        if let Some(pointer_location) = self.touch_location_transformed(&evt) {
            let tool = evt.tool();
            tablet_seat.add_tool::<Self>(self, dh, &tool);

            let pointer = self.pointer.clone();
            let under = self.surface_under(pointer_location);
            let tablet = tablet_seat.get_tablet(&TabletDescriptor::from(&evt.device()));
            let tool = tablet_seat.get_tool(&tool);

            pointer.motion(
                self,
                under.clone(),
                &MotionEvent {
                    location: pointer_location,
                    serial: SCOUNTER.next_serial(),
                    time: evt.time_msec(),
                },
            );
            pointer.frame(self);

            if let (Some(under), Some(tablet), Some(tool)) = (
                under.and_then(|(f, loc)| f.wl_surface().map(|s| (s.into_owned(), loc))),
                tablet,
                tool,
            ) {
                match evt.state() {
                    ProximityState::In => tool.proximity_in(
                        pointer_location,
                        under,
                        &tablet,
                        SCOUNTER.next_serial(),
                        evt.time_msec(),
                    ),
                    ProximityState::Out => tool.proximity_out(evt.time_msec()),
                }
            }
        }
    }

    fn on_tablet_tool_tip<B: InputBackend>(&mut self, evt: B::TabletToolTipEvent) {
        let tool = self.seat.tablet_seat().get_tool(&evt.tool());

        if let Some(tool) = tool {
            match evt.tip_state() {
                TabletToolTipState::Down => {
                    let serial = SCOUNTER.next_serial();
                    tool.tip_down(serial, evt.time_msec());

                    // change the keyboard focus
                    self.update_keyboard_focus(self.pointer.current_location(), serial);
                }
                TabletToolTipState::Up => {
                    tool.tip_up(evt.time_msec());
                }
            }
        }
    }

    fn on_tablet_button<B: InputBackend>(&mut self, evt: B::TabletToolButtonEvent) {
        let tool = self.seat.tablet_seat().get_tool(&evt.tool());

        if let Some(tool) = tool {
            tool.button(
                evt.button(),
                evt.button_state(),
                SCOUNTER.next_serial(),
                evt.time_msec(),
            );
        }
    }

    fn on_gesture_swipe_begin<B: InputBackend>(&mut self, evt: B::GestureSwipeBeginEvent) {
        let serial = SCOUNTER.next_serial();
        let pointer = self.pointer.clone();
        pointer.gesture_swipe_begin(
            self,
            &GestureSwipeBeginEvent {
                serial,
                time: evt.time_msec(),
                fingers: evt.fingers(),
            },
        );
    }

    fn on_gesture_swipe_update<B: InputBackend>(&mut self, evt: B::GestureSwipeUpdateEvent) {
        let pointer = self.pointer.clone();
        pointer.gesture_swipe_update(
            self,
            &GestureSwipeUpdateEvent {
                time: evt.time_msec(),
                delta: evt.delta(),
            },
        );
    }

    fn on_gesture_swipe_end<B: InputBackend>(&mut self, evt: B::GestureSwipeEndEvent) {
        let serial = SCOUNTER.next_serial();
        let pointer = self.pointer.clone();
        pointer.gesture_swipe_end(
            self,
            &GestureSwipeEndEvent {
                serial,
                time: evt.time_msec(),
                cancelled: evt.cancelled(),
            },
        );
    }

    fn on_gesture_pinch_begin<B: InputBackend>(&mut self, evt: B::GesturePinchBeginEvent) {
        let serial = SCOUNTER.next_serial();
        let pointer = self.pointer.clone();
        pointer.gesture_pinch_begin(
            self,
            &GesturePinchBeginEvent {
                serial,
                time: evt.time_msec(),
                fingers: evt.fingers(),
            },
        );
    }

    fn on_gesture_pinch_update<B: InputBackend>(&mut self, evt: B::GesturePinchUpdateEvent) {
        let pointer = self.pointer.clone();
        pointer.gesture_pinch_update(
            self,
            &GesturePinchUpdateEvent {
                time: evt.time_msec(),
                delta: evt.delta(),
                scale: evt.scale(),
                rotation: evt.rotation(),
            },
        );
    }

    fn on_gesture_pinch_end<B: InputBackend>(&mut self, evt: B::GesturePinchEndEvent) {
        let serial = SCOUNTER.next_serial();
        let pointer = self.pointer.clone();
        pointer.gesture_pinch_end(
            self,
            &GesturePinchEndEvent {
                serial,
                time: evt.time_msec(),
                cancelled: evt.cancelled(),
            },
        );
    }

    fn on_gesture_hold_begin<B: InputBackend>(&mut self, evt: B::GestureHoldBeginEvent) {
        let serial = SCOUNTER.next_serial();
        let pointer = self.pointer.clone();
        pointer.gesture_hold_begin(
            self,
            &GestureHoldBeginEvent {
                serial,
                time: evt.time_msec(),
                fingers: evt.fingers(),
            },
        );
    }

    fn on_gesture_hold_end<B: InputBackend>(&mut self, evt: B::GestureHoldEndEvent) {
        let serial = SCOUNTER.next_serial();
        let pointer = self.pointer.clone();
        pointer.gesture_hold_end(
            self,
            &GestureHoldEndEvent {
                serial,
                time: evt.time_msec(),
                cancelled: evt.cancelled(),
            },
        );
    }

    fn touch_location_transformed<B: InputBackend, E: AbsolutePositionEvent<B>>(
        &self,
        evt: &E,
    ) -> Option<Point<f64, Logical>> {
        let output = self
            .space
            .outputs()
            .find(|output| output.name().starts_with("eDP"))
            .or_else(|| self.space.outputs().next());

        let output = output?;
        let output_geometry = self.space.output_geometry(output)?;

        let transform = output.current_transform();
        let size = transform.invert().transform_size(output_geometry.size);
        Some(
            transform.transform_point_in(evt.position_transformed(size), &size.to_f64())
                + output_geometry.loc.to_f64(),
        )
    }

    fn on_touch_down<B: InputBackend>(&mut self, evt: B::TouchDownEvent) {
        let Some(handle) = self.seat.get_touch() else {
            return;
        };

        let Some(touch_location) = self.touch_location_transformed(&evt) else {
            return;
        };

        let serial = SCOUNTER.next_serial();
        self.update_keyboard_focus(touch_location, serial);

        let under = self.surface_under(touch_location);
        handle.down(
            self,
            under,
            &DownEvent {
                slot: evt.slot(),
                location: touch_location,
                serial,
                time: evt.time_msec(),
            },
        );
    }
    fn on_touch_up<B: InputBackend>(&mut self, evt: B::TouchUpEvent) {
        let Some(handle) = self.seat.get_touch() else {
            return;
        };
        let serial = SCOUNTER.next_serial();
        handle.up(
            self,
            &UpEvent {
                slot: evt.slot(),
                serial,
                time: evt.time_msec(),
            },
        )
    }
    fn on_touch_motion<B: InputBackend>(&mut self, evt: B::TouchMotionEvent) {
        let Some(handle) = self.seat.get_touch() else {
            return;
        };
        let Some(touch_location) = self.touch_location_transformed(&evt) else {
            return;
        };

        let under = self.surface_under(touch_location);
        handle.motion(
            self,
            under,
            &smithay::input::touch::MotionEvent {
                slot: evt.slot(),
                location: touch_location,
                time: evt.time_msec(),
            },
        );
    }
    fn on_touch_frame<B: InputBackend>(&mut self, _evt: B::TouchFrameEvent) {
        let Some(handle) = self.seat.get_touch() else {
            return;
        };
        handle.frame(self);
    }
    fn on_touch_cancel<B: InputBackend>(&mut self, _evt: B::TouchCancelEvent) {
        let Some(handle) = self.seat.get_touch() else {
            return;
        };
        handle.cancel(self);
    }

    fn clamp_coords(&self, pos: Point<f64, Logical>) -> Point<f64, Logical> {
        if self.space.outputs().next().is_none() {
            return pos;
        }

        let (pos_x, pos_y) = pos.into();
        let max_x = self.space.outputs().fold(0, |acc, o| {
            acc + self.space.output_geometry(o).unwrap().size.w
        });
        let clamped_x = pos_x.clamp(0.0, max_x as f64);
        let max_y = self
            .space
            .outputs()
            .find(|o| {
                let geo = self.space.output_geometry(o).unwrap();
                geo.contains((clamped_x as i32, 0))
            })
            .map(|o| self.space.output_geometry(o).unwrap().size.h);

        if let Some(max_y) = max_y {
            let clamped_y = pos_y.clamp(0.0, max_y as f64);
            (clamped_x, clamped_y).into()
        } else {
            (clamped_x, pos_y).into()
        }
    }
}

/// Modifier keysyms that can be tapped on their own.
fn is_modifier(sym: Keysym) -> bool {
    matches!(
        sym.raw(),
        xkb::KEY_Super_L
            | xkb::KEY_Super_R
            | xkb::KEY_Alt_L
            | xkb::KEY_Alt_R
            | xkb::KEY_Control_L
            | xkb::KEY_Control_R
            | xkb::KEY_Shift_L
            | xkb::KEY_Shift_R
    )
}
