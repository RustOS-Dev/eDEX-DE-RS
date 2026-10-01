//! Glue between the window model ([`crate::wm`]) and Smithay: placing windows in the `Space`,
//! configuring them, keyboard focus, and the control-socket events that follow every change.

use std::time::Duration;

use comp_proto::{Rect, Snapshot, WindowId, WindowMode};
use smithay::{
    desktop::{layer_map_for_output, WindowSurfaceType},
    output::Output,
    reexports::{
        wayland_protocols::xdg::shell::server::xdg_toplevel,
        wayland_server::{protocol::wl_surface::WlSurface, Resource},
    },
    utils::{IsAlive, Logical, Point, Rectangle, SERIAL_COUNTER},
    wayland::{
        compositor::with_states,
        shell::{
            wlr_layer::{KeyboardInteractivity, Layer},
            xdg::XdgToplevelSurfaceData,
        },
    },
};
use tracing::debug;

use crate::{
    focus::KeyboardFocusTarget,
    shell::{FullscreenSurface, WindowElement},
    state::{Backend, EdexState},
    wm::{OutputInfo, WindowInfo},
};

/// The id edex-comp gave a window, stored in its user data.
#[derive(Debug, Clone, Copy)]
pub struct ManagedId(pub WindowId);

pub fn to_rect(r: Rectangle<i32, Logical>) -> Rect {
    Rect::new(r.loc.x, r.loc.y, r.size.w, r.size.h)
}

pub fn from_rect(r: Rect) -> Rectangle<i32, Logical> {
    Rectangle::new((r.x, r.y).into(), (r.w, r.h).into())
}

impl<B: Backend + 'static> EdexState<B> {
    /// Take a new toplevel under management and lay it out.
    pub fn manage_window(&mut self, window: WindowElement, info: WindowInfo) -> WindowId {
        let title = info.title.clone();
        let app_id = info.app_id.clone();
        let id = self.wm.add_window(window.clone(), info);
        window.user_data().insert_if_missing(|| ManagedId(id));
        let handle = self
            .foreign_toplevel_state
            .new_toplevel::<Self>(&title, &app_id);
        self.foreign_toplevels.insert(id, handle);
        debug!(id, app_id, "managing window");
        self.mark_layout();
        id
    }

    pub fn unmanage_window(&mut self, window: &WindowElement) {
        if let Some(id) = self.wm.id_of(window) {
            self.wm.remove_window(id);
            if let Some(handle) = self.foreign_toplevels.remove(&id) {
                self.foreign_toplevel_state.remove_toplevel(&handle);
            }
        }
        self.space.unmap_elem(window);
        for output in self.space.outputs() {
            if let Some(fs) = output.user_data().get::<FullscreenSurface>() {
                if fs.get().as_ref() == Some(window) {
                    fs.clear();
                }
            }
        }
        self.mark_layout();
    }

    /// Any managed window (shown or not) or unmanaged X11 popup whose root surface is `surface`.
    pub fn window_for_surface(&self, surface: &WlSurface) -> Option<WindowElement> {
        self.wm
            .handles()
            .map(|(_, w)| w)
            .find(|w| w.wl_surface().as_deref() == Some(surface))
            .cloned()
            .or_else(|| {
                self.space
                    .elements()
                    .find(|w| w.wl_surface().as_deref() == Some(surface))
                    .cloned()
            })
    }

    pub fn window_id(&self, window: &WindowElement) -> Option<WindowId> {
        window.user_data().get::<ManagedId>().map(|m| m.0)
    }

    pub fn mark_layout(&mut self) {
        self.layout_dirty = true;
    }

    /// Run once per event-loop iteration: apply pending layout changes, keep focus valid and
    /// tell subscribers what changed.
    pub fn flush(&mut self) {
        if self.layout_dirty {
            self.layout_dirty = false;
            self.apply_layout();
        }
        self.ensure_focus();
        self.emit_events();
    }

    /// Register an output with the window model (after it is mapped in the space).
    pub fn wm_output_added(&mut self, output: &Output) {
        let Some(geo) = self.space.output_geometry(output) else {
            return;
        };
        let mode = output.current_mode();
        let modes = output
            .modes()
            .iter()
            .map(|m| {
                format!(
                    "{}x{}@{}",
                    m.size.w,
                    m.size.h,
                    (m.refresh as f32 / 1000.0).round()
                )
            })
            .collect();
        let props = output.physical_properties();
        self.wm.add_output(OutputInfo {
            name: output.name(),
            description: format!("{} {}", props.make, props.model),
            geometry: to_rect(geo),
            scale: output.current_scale().fractional_scale() as f32,
            transform: transform_index(output.current_transform()),
            refresh_rate: mode.map(|m| m.refresh as f32 / 1000.0).unwrap_or(0.0),
            mode_px: mode
                .map(|m| (m.size.w.max(0) as u32, m.size.h.max(0) as u32))
                .unwrap_or_default(),
            modes,
        });
        self.mark_layout();
    }

    pub fn wm_output_removed(&mut self, output: &Output) {
        self.wm.remove_output(&output.name());
        self.mark_layout();
    }

    /// Re-read every output's geometry (after a mode, scale or position change).
    pub fn wm_outputs_changed(&mut self) {
        let outputs: Vec<Output> = self.space.outputs().cloned().collect();
        for o in &outputs {
            self.wm_output_added(o);
        }
    }

    fn apply_layout(&mut self) {
        let mut placements = self.wm.arrange();
        // Map tiled windows first so floating and fullscreen ones end up on top.
        placements.sort_by_key(|p| (p.on_top, p.activated));
        let border = self.config.border;
        let (active, inactive) = (self.config.border_active, self.config.border_inactive);
        let mut fullscreen: Vec<(String, WindowElement)> = Vec::new();

        for p in &placements {
            let window = &p.handle;
            if !p.visible {
                if self.space.element_location(window).is_some() {
                    self.space.unmap_elem(window);
                }
                continue;
            }
            let rect = from_rect(p.rect);
            let draws_border = border > 0 && p.mode != WindowMode::Fullscreen;
            window.set_border(
                if draws_border { border } else { 0 },
                if p.activated { active } else { inactive },
            );
            configure(window, rect, p.mode, p.activated, p.tiled);
            self.space.map_element(window.clone(), rect.loc, false);
            if p.mode == WindowMode::Fullscreen {
                if let Some(name) = self.wm.output_of_window(p.id) {
                    fullscreen.push((name.to_string(), window.clone()));
                }
            }
        }
        for output in self.space.outputs() {
            output
                .user_data()
                .insert_if_missing(FullscreenSurface::default);
            let fs = output.user_data().get::<FullscreenSurface>().unwrap();
            match fullscreen.iter().find(|(name, _)| *name == output.name()) {
                Some((_, w)) => fs.set(w.clone()),
                None => {
                    fs.clear();
                }
            }
        }
        // Keyboard focus follows the model unless something else holds it on purpose.
        if !self.lock.locked && !self.exclusive_layer_focused() {
            let keyboard = self.seat.get_keyboard().unwrap();
            let wanted = self.wm.focused().and_then(|id| self.wm.handle(id).cloned());
            if let Some(w) = wanted {
                if keyboard.current_focus() != Some(KeyboardFocusTarget::from(w.clone())) {
                    if let Some(x) = w.0.x11_surface() {
                        if let Some(xwm) = self.xwm.as_mut() {
                            let _ = xwm.raise_window(x);
                        }
                    }
                    keyboard.set_focus(self, Some(w.into()), SERIAL_COUNTER.next_serial());
                }
            }
        }
        for (id, handle) in &self.foreign_toplevels {
            if let Some(w) = self.wm.windows().into_iter().find(|w| w.id == *id) {
                if handle.title() != w.title || handle.app_id() != w.app_id {
                    handle.send_title(&w.title);
                    handle.send_app_id(&w.app_id);
                    handle.send_done();
                }
            }
        }
    }

    /// True when a top/overlay layer surface with exclusive keyboard interactivity (the shell's
    /// launcher, settings, power menu) has the keyboard.
    pub fn exclusive_layer_focused(&self) -> bool {
        let Some(KeyboardFocusTarget::LayerSurface(layer)) =
            self.seat.get_keyboard().unwrap().current_focus()
        else {
            return false;
        };
        layer.alive()
            && matches!(layer.layer(), Layer::Top | Layer::Overlay)
            && layer.cached_state().keyboard_interactivity == KeyboardInteractivity::Exclusive
    }

    /// When the focused surface went away, give the keyboard to whatever had it before an
    /// overlay took it, else the focused window, else the shell canvas.
    fn ensure_focus(&mut self) {
        if self.lock.locked {
            return;
        }
        let keyboard = self.seat.get_keyboard().unwrap();
        let current = keyboard.current_focus();
        if current
            .as_ref()
            .is_some_and(|f| focus_alive_and_shown(self, f))
        {
            return;
        }
        let next = self
            .prev_focus
            .take()
            .filter(|f| focus_alive_and_shown(self, f))
            .or_else(|| {
                self.wm
                    .focused()
                    .and_then(|id| self.wm.handle(id).cloned())
                    .map(KeyboardFocusTarget::from)
            })
            .or_else(|| self.shell_canvas_focus());
        if next != current {
            keyboard.set_focus(self, next, SERIAL_COUNTER.next_serial());
        }
    }

    /// The shell's background canvas on the focused output, if it takes keyboard focus.
    pub fn shell_canvas_focus(&self) -> Option<KeyboardFocusTarget> {
        let name = self.wm.focused_output_name()?.to_string();
        let output = self.space.outputs().find(|o| o.name() == name)?;
        let map = layer_map_for_output(output);
        let layer = map
            .layers_on(Layer::Background)
            .chain(map.layers_on(Layer::Bottom))
            .find(|l| l.can_receive_keyboard_focus())
            .cloned()?;
        Some(KeyboardFocusTarget::LayerSurface(layer))
    }

    /// Called from `SeatHandler::focus_changed`.
    pub fn on_keyboard_focus(&mut self, target: Option<&KeyboardFocusTarget>) {
        match target {
            Some(KeyboardFocusTarget::Window(w)) => {
                if let Some(id) = w.user_data().get::<ManagedId>().map(|m| m.0) {
                    if self.wm.focused() != Some(id) {
                        self.wm.focus_window(id);
                        self.mark_layout();
                    }
                }
                self.last_regular_focus = target.cloned();
            }
            Some(KeyboardFocusTarget::LayerSurface(l))
                if matches!(l.layer(), Layer::Top | Layer::Overlay) =>
            {
                // An overlay took the keyboard: remember where to return.
                if self.prev_focus.is_none() {
                    self.prev_focus = self.last_regular_focus.clone();
                }
            }
            Some(KeyboardFocusTarget::LayerSurface(_)) => {
                // The shell canvas: windows lose focus (the terminal is typing).
                if self.wm.focused().is_some() {
                    self.wm.unfocus();
                    self.mark_layout();
                }
                self.last_regular_focus = target.cloned();
                self.prev_focus = None;
            }
            Some(KeyboardFocusTarget::Popup(_))
            | Some(KeyboardFocusTarget::LockSurface(_))
            | None => {}
        }
    }

    pub fn snapshot(&self) -> Snapshot {
        Snapshot {
            version: env!("CARGO_PKG_VERSION").to_string(),
            monitors: self.wm.monitors(),
            workspaces: self.wm.workspaces(),
            windows: self.wm.windows(),
            focused: self.wm.focused(),
            keyboard_layout: self.keyboard_layout.clone(),
            locked: self.lock.locked,
        }
    }

    fn emit_events(&mut self) {
        let Some(control) = self.control.as_mut() else {
            return;
        };
        if !control.has_subscribers() {
            self.last_snapshot = Snapshot {
                version: String::new(),
                ..Default::default()
            };
            return;
        }
        let now = Snapshot {
            version: env!("CARGO_PKG_VERSION").to_string(),
            monitors: self.wm.monitors(),
            workspaces: self.wm.workspaces(),
            windows: self.wm.windows(),
            focused: self.wm.focused(),
            keyboard_layout: self.keyboard_layout.clone(),
            locked: self.lock.locked,
        };
        let events = crate::ipc::diff(&self.last_snapshot, &now);
        control.broadcast(&events);
        self.last_snapshot = now;
    }

    /// Window title and app id after a commit or an X11 property change.
    pub fn refresh_window_info(&mut self, window: &WindowElement) {
        let Some(id) = self.window_id(window) else {
            return;
        };
        let (title, app_id) = window_title_app_id(window);
        let a = self.wm.set_title(id, &title);
        let b = self.wm.set_app_id(id, &app_id);
        if a || b {
            self.mark_layout();
        }
    }

    /// Interactive moves and resizes of floating windows end here.
    pub fn window_geometry_settled(&mut self, window: &WindowElement) {
        let (Some(id), Some(geo)) = (self.window_id(window), self.space.element_geometry(window))
        else {
            return;
        };
        self.wm.set_float_rect(id, to_rect(geo));
    }

    pub fn idle_poll_interval() -> Duration {
        Duration::from_secs(1)
    }
}

fn focus_alive_and_shown<B: Backend + 'static>(
    state: &EdexState<B>,
    target: &KeyboardFocusTarget,
) -> bool {
    match target {
        KeyboardFocusTarget::Window(w) => {
            w.alive()
                && state
                    .space
                    .elements()
                    .any(|e| e.0 == *w && state.space.element_location(e).is_some())
        }
        KeyboardFocusTarget::LayerSurface(l) => {
            l.alive()
                && state.space.outputs().any(|o| {
                    layer_map_for_output(o)
                        .layer_for_surface(l.wl_surface(), WindowSurfaceType::TOPLEVEL)
                        .is_some()
                })
        }
        KeyboardFocusTarget::Popup(p) => p.alive(),
        KeyboardFocusTarget::LockSurface(s) => s.alive(),
    }
}

/// Title and app id (or WM_CLASS) of a window.
pub fn window_title_app_id(window: &WindowElement) -> (String, String) {
    if let Some(toplevel) = window.0.toplevel() {
        with_states(toplevel.wl_surface(), |states| {
            let data = states
                .data_map
                .get::<XdgToplevelSurfaceData>()
                .unwrap()
                .lock()
                .unwrap();
            (
                data.title.clone().unwrap_or_default(),
                data.app_id.clone().unwrap_or_default(),
            )
        })
    } else if let Some(x) = window.0.x11_surface() {
        (x.title(), x.class())
    } else {
        Default::default()
    }
}

/// Send the size and states for a placement.
fn configure(
    window: &WindowElement,
    rect: Rectangle<i32, Logical>,
    mode: WindowMode,
    activated: bool,
    tiled: bool,
) {
    if let Some(toplevel) = window.0.toplevel() {
        toplevel.with_pending_state(|state| {
            use xdg_toplevel::State;
            let set = |states: &mut smithay::wayland::shell::xdg::ToplevelStateSet,
                       s: State,
                       on: bool| {
                if on {
                    states.set(s);
                } else {
                    states.unset(s);
                }
            };
            set(&mut state.states, State::Activated, activated);
            set(
                &mut state.states,
                State::Maximized,
                mode == WindowMode::Maximized,
            );
            set(
                &mut state.states,
                State::Fullscreen,
                mode == WindowMode::Fullscreen,
            );
            for edge in [
                State::TiledLeft,
                State::TiledRight,
                State::TiledTop,
                State::TiledBottom,
            ] {
                set(&mut state.states, edge, tiled);
            }
            state.size = if mode == WindowMode::Floating {
                state.size.or(Some(rect.size))
            } else {
                Some(rect.size)
            };
            state.bounds = Some(rect.size);
        });
        if toplevel.is_initial_configure_sent() {
            toplevel.send_pending_configure();
        }
    } else if let Some(x) = window.0.x11_surface() {
        let _ = x.set_activated(activated);
        let _ = x.set_maximized(mode == WindowMode::Maximized);
        let _ = x.set_fullscreen(mode == WindowMode::Fullscreen);
        let size = if mode == WindowMode::Floating && !x.geometry().size.is_empty() {
            x.geometry().size
        } else {
            rect.size
        };
        let _ = x.configure(Rectangle::new(rect.loc, size));
    }
}

/// wl_output transforms in protocol order.
const TRANSFORMS: [smithay::utils::Transform; 8] = [
    smithay::utils::Transform::Normal,
    smithay::utils::Transform::_90,
    smithay::utils::Transform::_180,
    smithay::utils::Transform::_270,
    smithay::utils::Transform::Flipped,
    smithay::utils::Transform::Flipped90,
    smithay::utils::Transform::Flipped180,
    smithay::utils::Transform::Flipped270,
];

pub fn transform_index(t: smithay::utils::Transform) -> u32 {
    TRANSFORMS.iter().position(|x| *x == t).unwrap_or(0) as u32
}

pub fn transform_from_index(i: u32) -> smithay::utils::Transform {
    TRANSFORMS
        .get(i as usize)
        .copied()
        .unwrap_or(smithay::utils::Transform::Normal)
}

/// Where the pointer is, as an integer point (for output focus).
pub fn point_i32(p: Point<f64, Logical>) -> (i32, i32) {
    (p.x.round() as i32, p.y.round() as i32)
}

/// Whether a wl_surface still belongs to a live client (used before sending to it).
pub fn surface_alive(s: &WlSurface) -> bool {
    s.is_alive()
}
