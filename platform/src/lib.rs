//! Wayland client platform layer for eDEX-DE.
//!
//! `Platform` owns the Wayland connection, the calloop event loop registration, all
//! sctk protocol states and every surface the shell creates. Handlers translate Wayland
//! events into [`PlatformEvent`]s which the application drains after each dispatch.

pub mod events;
pub mod surface;

use std::{
    collections::{HashMap, VecDeque},
    io::{Read, Write},
    ptr::NonNull,
    time::Duration,
};

use anyhow::{anyhow, Context, Result};
use calloop::{channel, EventLoop, LoopHandle};
use calloop_wayland_source::WaylandSource;
use raw_window_handle::{
    RawDisplayHandle, RawWindowHandle, WaylandDisplayHandle, WaylandWindowHandle,
};
use smithay_client_toolkit::{
    compositor::{CompositorHandler, CompositorState, FrameCallbackData, Region},
    data_device_manager::{
        data_device::{DataDevice, DataDeviceHandler},
        data_offer::{DataOfferHandler, DragOffer},
        data_source::{CopyPasteSource, DataSourceHandler},
        DataDeviceManagerState, WritePipe,
    },
    delegate_registry,
    dispatch2::Dispatch2,
    output::{OutputHandler, OutputState},
    registry::{ProvidesRegistryState, RegistryState},
    registry_handlers,
    seat::{
        keyboard::{KeyEvent, KeyboardHandler, Keysym, Modifiers as SctkModifiers, RawModifiers},
        pointer::{
            AxisScroll, CursorIcon, PointerEvent, PointerEventKind, PointerHandler, ThemeSpec,
            ThemedPointer,
        },
        Capability, SeatHandler, SeatState,
    },
    session_lock::{
        SessionLock, SessionLockHandler, SessionLockState, SessionLockSurface,
        SessionLockSurfaceConfigure,
    },
    shell::{
        wlr_layer::{
            Anchor, KeyboardInteractivity, Layer, LayerShell, LayerShellHandler, LayerSurface,
            LayerSurfaceConfigure,
        },
        xdg::{
            window::{Window, WindowConfigure, WindowDecorations, WindowHandler},
            XdgShell,
        },
        WaylandSurface,
    },
    shm::{Shm, ShmHandler},
};
use tracing::{debug, info, warn};
use wayland_client::{
    globals::{registry_queue_init, GlobalList},
    protocol::{
        wl_data_device::WlDataDevice, wl_data_device_manager::DndAction,
        wl_data_source::WlDataSource, wl_keyboard, wl_output, wl_pointer, wl_seat, wl_surface,
    },
    Connection, Proxy, QueueHandle,
};
use wayland_protocols::wp::{
    fractional_scale::v1::client::{
        wp_fractional_scale_manager_v1::WpFractionalScaleManagerV1,
        wp_fractional_scale_v1::{self, WpFractionalScaleV1},
    },
    viewporter::client::{wp_viewport::WpViewport, wp_viewporter::WpViewporter},
};

pub use events::{button, KeyInput, Modifiers, OutputInfo, PlatformEvent};
pub use smithay_client_toolkit::seat::pointer::CursorIcon as Cursor;
pub use surface::SurfaceRole;
use surface::{SurfaceEntry, SurfaceKind};

/// Identifier of a surface created through the platform.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct SurfaceId(pub u64);

/// Identifier of an output (the `wl_output` global name).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct OutputId(pub u32);

const TEXT_MIME: &str = "text/plain;charset=utf-8";
const TEXT_MIME_PLAIN: &str = "text/plain";

/// Description of a layer surface to create.
#[derive(Clone, Debug)]
pub struct LayerSpec {
    pub role: SurfaceRole,
    pub output: Option<OutputId>,
    pub layer: Layer,
    pub anchor: Anchor,
    /// Requested size in logical pixels; 0 means "fill the anchored axis".
    pub size: (u32, u32),
    pub exclusive_zone: i32,
    pub keyboard: KeyboardInteractivity,
    pub namespace: String,
    /// When false the surface has an empty input region and pointer events pass through.
    pub accepts_input: bool,
    pub margin: (i32, i32, i32, i32),
}

impl LayerSpec {
    pub fn canvas(output: OutputId) -> Self {
        Self {
            role: SurfaceRole::Canvas,
            output: Some(output),
            layer: Layer::Background,
            anchor: Anchor::TOP | Anchor::BOTTOM | Anchor::LEFT | Anchor::RIGHT,
            size: (0, 0),
            exclusive_zone: -1,
            keyboard: KeyboardInteractivity::OnDemand,
            namespace: "edex-de:canvas".into(),
            accepts_input: true,
            margin: (0, 0, 0, 0),
        }
    }

    pub fn overlay(output: OutputId) -> Self {
        Self {
            role: SurfaceRole::Overlay,
            output: Some(output),
            layer: Layer::Overlay,
            anchor: Anchor::TOP | Anchor::BOTTOM | Anchor::LEFT | Anchor::RIGHT,
            size: (0, 0),
            exclusive_zone: -1,
            keyboard: KeyboardInteractivity::Exclusive,
            namespace: "edex-de:overlay".into(),
            accepts_input: true,
            margin: (0, 0, 0, 0),
        }
    }

    pub fn toast(output: OutputId, width: u32, height: u32, top_margin: i32) -> Self {
        Self {
            role: SurfaceRole::Toast,
            output: Some(output),
            layer: Layer::Overlay,
            anchor: Anchor::TOP | Anchor::RIGHT,
            size: (width, height),
            exclusive_zone: -1,
            keyboard: KeyboardInteractivity::None,
            namespace: "edex-de:toast".into(),
            accepts_input: true,
            margin: (top_margin, 0, 0, 0),
        }
    }

    /// Bar at logical position (x, y) with the given size, above windows.
    pub fn strip(output: OutputId, x: i32, y: i32, width: u32, height: u32) -> Self {
        Self {
            role: SurfaceRole::Strip,
            output: Some(output),
            layer: Layer::Top,
            anchor: Anchor::TOP | Anchor::LEFT,
            size: (width.max(1), height.max(1)),
            exclusive_zone: -1,
            keyboard: KeyboardInteractivity::None,
            namespace: "edex-de:strip".into(),
            accepts_input: true,
            margin: (y, 0, 0, x),
        }
    }
}

struct SeatEntry {
    seat: wl_seat::WlSeat,
    keyboard: Option<wl_keyboard::WlKeyboard>,
    pointer: Option<ThemedPointer>,
    data_device: Option<DataDevice>,
    last_serial: u32,
}

enum Internal {
    Paste { text: String, primary: bool },
}

/// The Wayland platform state. `E` is the application's own event type, delivered through
/// [`PlatformEvent::App`] when application-registered calloop sources fire.
pub struct Platform<E: 'static> {
    pub conn: Connection,
    qh: QueueHandle<Self>,
    pub loop_handle: LoopHandle<'static, Self>,
    registry_state: RegistryState,
    output_state: OutputState,
    seat_state: SeatState,
    compositor: CompositorState,
    shm: Shm,
    layer_shell: Option<LayerShell>,
    xdg_shell: Option<XdgShell>,
    viewporter: Option<WpViewporter>,
    fractional_manager: Option<WpFractionalScaleManagerV1>,
    data_device_manager: Option<DataDeviceManagerState>,
    session_lock_state: SessionLockState,
    session_lock: Option<SessionLock>,
    surfaces: HashMap<SurfaceId, SurfaceEntry>,
    by_wl: HashMap<wayland_client::backend::ObjectId, SurfaceId>,
    next_surface: u64,
    outputs: HashMap<OutputId, wl_output::WlOutput>,
    seats: Vec<SeatEntry>,
    keyboard_focus: Option<SurfaceId>,
    pointer_focus: Option<SurfaceId>,
    pointer_position: (f64, f64),
    modifiers: Modifiers,
    copy_source: Option<CopyPasteSource>,
    copy_text: String,
    internal_tx: channel::Sender<Internal>,
    events: VecDeque<PlatformEvent<E>>,
    exit: bool,
}

impl<E: 'static> Platform<E> {
    /// Connect to the Wayland display named by the environment and build the event loop.
    pub fn new() -> Result<(EventLoop<'static, Self>, Self)> {
        let conn =
            Connection::connect_to_env().context("failed to connect to the Wayland display")?;
        let (globals, event_queue) =
            registry_queue_init::<Self>(&conn).context("failed to initialise the registry")?;
        let qh = event_queue.handle();
        let event_loop: EventLoop<'static, Self> =
            EventLoop::try_new().context("failed to create calloop event loop")?;
        let loop_handle = event_loop.handle();
        WaylandSource::new(conn.clone(), event_queue)
            .insert(loop_handle.clone())
            .map_err(|e| anyhow!("failed to insert Wayland source: {e}"))?;

        let compositor = CompositorState::bind(&globals, &qh).context("wl_compositor missing")?;
        let shm = Shm::bind(&globals, &qh).context("wl_shm missing")?;
        let layer_shell = LayerShell::bind(&globals, &qh).ok();
        let xdg_shell = XdgShell::bind(&globals, &qh).ok();
        let viewporter = bind_optional::<WpViewporter, Self>(&globals, &qh, 1..=1);
        let fractional_manager =
            bind_optional::<WpFractionalScaleManagerV1, Self>(&globals, &qh, 1..=1);
        let data_device_manager = DataDeviceManagerState::bind(&globals, &qh).ok();
        let session_lock_state = SessionLockState::new(&globals, &qh);

        let (internal_tx, internal_rx) = channel::channel::<Internal>();
        loop_handle
            .insert_source(internal_rx, |event, _, state: &mut Self| {
                if let channel::Event::Msg(Internal::Paste { text, primary }) = event {
                    state
                        .events
                        .push_back(PlatformEvent::Paste { text, primary });
                }
            })
            .map_err(|e| anyhow!("failed to insert internal channel: {e}"))?;

        info!(
            layer_shell = layer_shell.is_some(),
            xdg_shell = xdg_shell.is_some(),
            viewporter = viewporter.is_some(),
            fractional_scale = fractional_manager.is_some(),
            data_device = data_device_manager.is_some(),
            "wayland globals bound"
        );

        let platform = Self {
            conn,
            qh: qh.clone(),
            loop_handle,
            registry_state: RegistryState::new(&globals),
            output_state: OutputState::new(&globals, &qh),
            seat_state: SeatState::new(&globals, &qh),
            compositor,
            shm,
            layer_shell,
            xdg_shell,
            viewporter,
            fractional_manager,
            data_device_manager,
            session_lock_state,
            session_lock: None,
            surfaces: HashMap::new(),
            by_wl: HashMap::new(),
            next_surface: 1,
            outputs: HashMap::new(),
            seats: Vec::new(),
            keyboard_focus: None,
            pointer_focus: None,
            pointer_position: (0.0, 0.0),
            modifiers: Modifiers::default(),
            copy_source: None,
            copy_text: String::new(),
            internal_tx,
            events: VecDeque::new(),
            exit: false,
        };
        Ok((event_loop, platform))
    }
}

fn bind_optional<I, D>(
    globals: &GlobalList,
    qh: &QueueHandle<D>,
    versions: std::ops::RangeInclusive<u32>,
) -> Option<I>
where
    I: Proxy + 'static,
    D: wayland_client::Dispatch<I, NoEvents> + 'static,
{
    globals.bind::<I, D, NoEvents>(qh, versions, NoEvents).ok()
}

/// User data for protocol objects that never emit events.
#[derive(Debug, Clone, Copy, Default)]
pub struct NoEvents;

impl<I: Proxy, D> Dispatch2<I, D> for NoEvents {
    fn event(&self, _: &mut D, _: &I, _: <I as Proxy>::Event, _: &Connection, _: &QueueHandle<D>) {}
}

/// User data of a `wp_fractional_scale_v1` object: the surface it belongs to.
#[derive(Debug, Clone, Copy)]
pub struct FractionalScaleData(SurfaceId);

impl<E: 'static> Dispatch2<WpFractionalScaleV1, Platform<E>> for FractionalScaleData {
    fn event(
        &self,
        state: &mut Platform<E>,
        _: &WpFractionalScaleV1,
        event: wp_fractional_scale_v1::Event,
        _: &Connection,
        _: &QueueHandle<Platform<E>>,
    ) {
        if let wp_fractional_scale_v1::Event::PreferredScale { scale } = event {
            let id = self.0;
            let changed = match state.surfaces.get_mut(&id) {
                Some(entry) if entry.scale120 != Some(scale) => {
                    entry.scale120 = Some(scale);
                    entry.configured
                }
                _ => false,
            };
            if changed {
                state.events.push_back(PlatformEvent::ScaleChanged {
                    surface: id,
                    scale: scale as f64 / 120.0,
                });
            }
        }
    }
}

impl<E: 'static> Platform<E> {
    /// Handle to push application events from calloop sources.
    pub fn push_app_event(&mut self, event: E) {
        self.events.push_back(PlatformEvent::App(event));
    }

    /// Drain the events produced by the last dispatch.
    pub fn drain_events(&mut self) -> VecDeque<PlatformEvent<E>> {
        std::mem::take(&mut self.events)
    }

    /// Whether the compositor asked us to exit (all surfaces closed / display gone).
    pub fn should_exit(&self) -> bool {
        self.exit
    }

    pub fn request_exit(&mut self) {
        self.exit = true;
    }

    /// Dispatch pending Wayland events and calloop sources, waiting up to `timeout`.
    pub fn dispatch(
        &mut self,
        event_loop: &mut EventLoop<'static, Self>,
        timeout: Option<Duration>,
    ) -> Result<()> {
        self.conn.flush().context("wayland flush failed")?;
        event_loop
            .dispatch(timeout, self)
            .map_err(|e| anyhow!("event loop dispatch failed: {e}"))?;
        Ok(())
    }

    pub fn has_layer_shell(&self) -> bool {
        self.layer_shell.is_some()
    }

    pub fn has_xdg_shell(&self) -> bool {
        self.xdg_shell.is_some()
    }

    pub fn outputs(&self) -> Vec<OutputInfo> {
        self.outputs
            .keys()
            .filter_map(|id| self.output_info(*id))
            .collect()
    }

    pub fn output_info(&self, id: OutputId) -> Option<OutputInfo> {
        let output = self.outputs.get(&id)?;
        let info = self.output_state.info(output)?;
        Some(convert_output_info(id, &info))
    }

    fn wl_output(&self, id: OutputId) -> Option<&wl_output::WlOutput> {
        self.outputs.get(&id)
    }

    fn alloc_id(&mut self) -> SurfaceId {
        let id = SurfaceId(self.next_surface);
        self.next_surface += 1;
        id
    }

    fn attach_scale_helpers(
        &mut self,
        id: SurfaceId,
        wl_surface: &wl_surface::WlSurface,
    ) -> (Option<WpViewport>, Option<WpFractionalScaleV1>) {
        let viewport = self
            .viewporter
            .as_ref()
            .map(|v| v.get_viewport(wl_surface, &self.qh, NoEvents));
        let fractional = self
            .fractional_manager
            .as_ref()
            .map(|f| f.get_fractional_scale(wl_surface, &self.qh, FractionalScaleData(id)));
        (viewport, fractional)
    }

    /// Create a layer-shell surface. The compositor answers with a `Configure` event; until
    /// then the surface must not be rendered.
    pub fn create_layer_surface(&mut self, spec: LayerSpec) -> Result<SurfaceId> {
        let id = self.alloc_id();
        let layer_shell = self
            .layer_shell
            .as_ref()
            .ok_or_else(|| anyhow!("compositor does not support zwlr_layer_shell_v1"))?;
        let wl_surface = self.compositor.create_surface(&self.qh);
        let output = spec.output.and_then(|o| self.wl_output(o).cloned());
        let layer = layer_shell.create_layer_surface(
            &self.qh,
            wl_surface.clone(),
            spec.layer,
            Some(spec.namespace.clone()),
            output.as_ref(),
        );
        layer.set_anchor(spec.anchor);
        layer.set_size(spec.size.0, spec.size.1);
        layer.set_exclusive_zone(spec.exclusive_zone);
        layer.set_keyboard_interactivity(spec.keyboard);
        layer.set_margin(spec.margin.0, spec.margin.1, spec.margin.2, spec.margin.3);
        if !spec.accepts_input {
            if let Ok(region) = Region::new(&self.compositor) {
                wl_surface.set_input_region(Some(region.wl_region()));
            }
        }
        let (viewport, fractional) = self.attach_scale_helpers(id, &wl_surface);
        layer.commit();
        self.by_wl.insert(wl_surface.id(), id);
        self.surfaces.insert(
            id,
            SurfaceEntry {
                wl_surface,
                kind: SurfaceKind::Layer(layer),
                role: spec.role,
                output: spec.output,
                viewport,
                fractional,
                scale120: None,
                int_scale: 1,
                logical_size: spec.size,
                configured: false,
                frame_pending: false,
            },
        );
        debug!(?id, role = ?spec.role, "layer surface created");
        Ok(id)
    }

    /// Create an xdg-toplevel window (fullscreen by default; used by the greeter).
    pub fn create_window(
        &mut self,
        title: &str,
        app_id: &str,
        fullscreen: bool,
    ) -> Result<SurfaceId> {
        let id = self.alloc_id();
        let xdg = self
            .xdg_shell
            .as_ref()
            .ok_or_else(|| anyhow!("compositor does not support xdg_wm_base"))?;
        let wl_surface = self.compositor.create_surface(&self.qh);
        let window = xdg.create_window(
            wl_surface.clone(),
            WindowDecorations::RequestServer,
            &self.qh,
        );
        window.set_title(title);
        window.set_app_id(app_id);
        window.set_min_size(Some((640, 480)));
        if fullscreen {
            window.set_fullscreen(None);
        }
        let (viewport, fractional) = self.attach_scale_helpers(id, &wl_surface);
        window.commit();
        self.by_wl.insert(wl_surface.id(), id);
        self.surfaces.insert(
            id,
            SurfaceEntry {
                wl_surface,
                kind: SurfaceKind::Window(window),
                role: SurfaceRole::Window,
                output: None,
                viewport,
                fractional,
                scale120: None,
                int_scale: 1,
                logical_size: (1280, 720),
                configured: false,
                frame_pending: false,
            },
        );
        Ok(id)
    }

    /// Lock the session (ext-session-lock) and cover every output with a lock surface. The
    /// surfaces arrive as `Configure` events like any other; a refused lock closes them.
    pub fn lock_session(&mut self) -> Result<Vec<SurfaceId>> {
        let lock = self
            .session_lock_state
            .lock(&self.qh)
            .map_err(|e| anyhow!("the compositor cannot lock the session: {e}"))?;
        let outputs: Vec<(OutputId, wl_output::WlOutput)> = self
            .outputs
            .iter()
            .map(|(id, o)| (*id, o.clone()))
            .collect();
        let mut ids = Vec::new();
        for (output_id, output) in outputs {
            let id = self.alloc_id();
            let wl_surface = self.compositor.create_surface(&self.qh);
            let (viewport, fractional) = self.attach_scale_helpers(id, &wl_surface);
            let surface = lock.create_lock_surface(wl_surface.clone(), &output, &self.qh);
            self.by_wl.insert(wl_surface.id(), id);
            self.surfaces.insert(
                id,
                SurfaceEntry {
                    wl_surface,
                    kind: SurfaceKind::Lock(surface),
                    role: SurfaceRole::Lock,
                    output: Some(output_id),
                    viewport,
                    fractional,
                    scale120: None,
                    int_scale: 1,
                    logical_size: (1280, 720),
                    configured: false,
                    frame_pending: false,
                },
            );
            ids.push(id);
        }
        self.session_lock = Some(lock);
        Ok(ids)
    }

    /// Unlock the session (after the password was checked) and drop the lock surfaces.
    pub fn unlock_session(&mut self) {
        if let Some(lock) = self.session_lock.take() {
            lock.unlock();
        }
        let ids: Vec<SurfaceId> = self
            .surfaces
            .iter()
            .filter(|(_, e)| e.role == SurfaceRole::Lock)
            .map(|(id, _)| *id)
            .collect();
        for id in ids {
            self.destroy_surface(id);
        }
        let _ = self.conn.flush();
    }

    pub fn is_locked(&self) -> bool {
        self.session_lock.as_ref().is_some_and(|l| l.is_locked())
    }

    pub fn destroy_surface(&mut self, id: SurfaceId) {
        if let Some(entry) = self.surfaces.remove(&id) {
            self.by_wl.remove(&entry.wl_surface.id());
            if let Some(f) = entry.fractional {
                f.destroy();
            }
            if let Some(v) = entry.viewport {
                v.destroy();
            }
            if self.keyboard_focus == Some(id) {
                self.keyboard_focus = None;
            }
            if self.pointer_focus == Some(id) {
                self.pointer_focus = None;
            }
            // Dropping `entry.kind` destroys the layer surface / window and the wl_surface.
        }
    }

    pub fn surface_role(&self, id: SurfaceId) -> Option<SurfaceRole> {
        self.surfaces.get(&id).map(|e| e.role)
    }

    pub fn surface_output(&self, id: SurfaceId) -> Option<OutputId> {
        self.surfaces.get(&id).and_then(|e| e.output)
    }

    pub fn surfaces_for_output(&self, output: OutputId) -> Vec<SurfaceId> {
        let mut ids: Vec<_> = self
            .surfaces
            .iter()
            .filter(|(_, e)| e.output == Some(output))
            .map(|(id, _)| *id)
            .collect();
        ids.sort();
        ids
    }

    pub fn is_configured(&self, id: SurfaceId) -> bool {
        self.surfaces
            .get(&id)
            .map(|e| e.configured)
            .unwrap_or(false)
    }

    /// Logical size of a configured surface.
    pub fn logical_size(&self, id: SurfaceId) -> Option<(u32, u32)> {
        self.surfaces
            .get(&id)
            .filter(|e| e.configured)
            .map(|e| e.logical_size)
    }

    pub fn scale(&self, id: SurfaceId) -> f64 {
        self.surfaces.get(&id).map(|e| e.scale()).unwrap_or(1.0)
    }

    /// Pixel size the swapchain should have for this surface.
    pub fn buffer_size(&self, id: SurfaceId) -> Option<(u32, u32)> {
        self.surfaces
            .get(&id)
            .filter(|e| e.configured)
            .map(|e| e.buffer_size())
    }

    /// Update the requested size of a layer surface.
    pub fn set_layer_size(&mut self, id: SurfaceId, width: u32, height: u32) {
        if let Some(entry) = self.surfaces.get_mut(&id) {
            if let SurfaceKind::Layer(layer) = &entry.kind {
                layer.set_size(width, height);
                layer.commit();
            }
        }
    }

    /// Move a layer surface: margins (top, right, bottom, left) from its anchored edges.
    pub fn set_layer_margin(&mut self, id: SurfaceId, margin: (i32, i32, i32, i32)) {
        if let Some(entry) = self.surfaces.get_mut(&id) {
            if let SurfaceKind::Layer(layer) = &entry.kind {
                layer.set_margin(margin.0, margin.1, margin.2, margin.3);
                layer.commit();
            }
        }
    }

    pub fn set_exclusive_zone(&mut self, id: SurfaceId, zone: i32) {
        if let Some(entry) = self.surfaces.get_mut(&id) {
            if let SurfaceKind::Layer(layer) = &entry.kind {
                layer.set_exclusive_zone(zone);
                layer.commit();
            }
        }
    }

    pub fn set_keyboard_interactivity(&mut self, id: SurfaceId, mode: KeyboardInteractivity) {
        if let Some(entry) = self.surfaces.get_mut(&id) {
            if let SurfaceKind::Layer(layer) = &entry.kind {
                layer.set_keyboard_interactivity(mode);
                layer.commit();
            }
        }
    }

    /// Whether any seat currently gives us a keyboard / pointer object.
    pub fn input_devices(&self) -> (bool, bool) {
        (
            self.seats.iter().any(|s| s.keyboard.is_some()),
            self.seats.iter().any(|s| s.pointer.is_some()),
        )
    }

    /// Ask for keyboard focus now (exclusive), e.g. for the canvas at login; undo with
    /// [`Platform::release_keyboard_grab`] once focus has arrived.
    pub fn grab_keyboard(&mut self, id: SurfaceId) {
        self.set_keyboard_interactivity(id, KeyboardInteractivity::Exclusive);
    }

    /// Back to on-demand focus: the surface keeps focus until the user clicks elsewhere.
    pub fn release_keyboard_grab(&mut self, id: SurfaceId) {
        self.set_keyboard_interactivity(id, KeyboardInteractivity::OnDemand);
    }

    pub fn set_layer(&mut self, id: SurfaceId, layer_kind: Layer) {
        if let Some(entry) = self.surfaces.get_mut(&id) {
            if let SurfaceKind::Layer(layer) = &entry.kind {
                layer.set_layer(layer_kind);
                layer.commit();
            }
        }
    }

    /// Request a frame callback for the surface. Must be called before the commit that
    /// presents the next frame (wgpu's present performs that commit).
    pub fn request_frame(&mut self, id: SurfaceId) -> bool {
        let Some(entry) = self.surfaces.get_mut(&id) else {
            return false;
        };
        if entry.frame_pending || !entry.configured {
            return false;
        }
        entry
            .wl_surface
            .frame(&self.qh, FrameCallbackData(entry.wl_surface.clone()));
        entry.frame_pending = true;
        true
    }

    /// Whether a frame callback is outstanding for the surface.
    pub fn frame_pending(&self, id: SurfaceId) -> bool {
        self.surfaces
            .get(&id)
            .map(|e| e.frame_pending)
            .unwrap_or(false)
    }

    /// Commit the surface without a new buffer (used after `request_frame` when nothing
    /// else commits, e.g. to keep a surface mapped).
    pub fn commit(&self, id: SurfaceId) {
        if let Some(entry) = self.surfaces.get(&id) {
            entry.wl_surface.commit();
        }
    }

    /// Raw handles for creating a GPU surface.
    pub fn raw_handles(&self, id: SurfaceId) -> Result<(RawDisplayHandle, RawWindowHandle)> {
        let entry = self
            .surfaces
            .get(&id)
            .ok_or_else(|| anyhow!("unknown surface {id:?}"))?;
        let display = RawDisplayHandle::Wayland(WaylandDisplayHandle::new(
            NonNull::new(self.conn.backend().display_ptr().cast())
                .ok_or_else(|| anyhow!("null wayland display pointer"))?,
        ));
        let window = RawWindowHandle::Wayland(WaylandWindowHandle::new(
            NonNull::new(entry.wl_surface.id().as_ptr().cast())
                .ok_or_else(|| anyhow!("null wl_surface pointer"))?,
        ));
        Ok((display, window))
    }

    pub fn set_cursor(&mut self, icon: CursorIcon) {
        let conn = self.conn.clone();
        for seat in &self.seats {
            if let Some(pointer) = &seat.pointer {
                let _ = pointer.set_cursor(&conn, icon);
            }
        }
    }

    pub fn modifiers(&self) -> Modifiers {
        self.modifiers
    }

    pub fn keyboard_focus(&self) -> Option<SurfaceId> {
        self.keyboard_focus
    }

    /// Offer `text` as the clipboard selection.
    pub fn copy_to_clipboard(&mut self, text: String) {
        let Some(manager) = &self.data_device_manager else {
            warn!("no data device manager; clipboard copy ignored");
            return;
        };
        let source = manager.create_copy_paste_source(&self.qh, [TEXT_MIME, TEXT_MIME_PLAIN]);
        for seat in &self.seats {
            if let Some(device) = &seat.data_device {
                source.set_selection(device, seat.last_serial);
            }
        }
        self.copy_text = text;
        self.copy_source = Some(source);
    }

    /// Ask for the current clipboard text; delivered later as `PlatformEvent::Paste`.
    pub fn request_paste(&mut self) {
        for seat in &self.seats {
            let Some(device) = &seat.data_device else {
                continue;
            };
            let Some(offer) = device.data().selection_offer() else {
                continue;
            };
            let mime = offer.with_mime_types(|mimes| {
                if mimes.iter().any(|m| m == TEXT_MIME) {
                    Some(TEXT_MIME.to_string())
                } else if mimes.iter().any(|m| m == TEXT_MIME_PLAIN) {
                    Some(TEXT_MIME_PLAIN.to_string())
                } else {
                    mimes.iter().find(|m| m.starts_with("text/")).cloned()
                }
            });
            let Some(mime) = mime else { continue };
            match offer.receive(mime) {
                Ok(mut pipe) => {
                    let tx = self.internal_tx.clone();
                    // Flush so the compositor sees the receive request before we block.
                    let _ = self.conn.flush();
                    std::thread::spawn(move || {
                        let mut buf = Vec::new();
                        if pipe.read_to_end(&mut buf).is_ok() {
                            let text = String::from_utf8_lossy(&buf).into_owned();
                            let _ = tx.send(Internal::Paste {
                                text,
                                primary: false,
                            });
                        }
                    });
                }
                Err(e) => warn!("clipboard receive failed: {e}"),
            }
            break;
        }
    }

    fn surface_id(&self, wl: &wl_surface::WlSurface) -> Option<SurfaceId> {
        self.by_wl.get(&wl.id()).copied()
    }

    fn convert_key(
        event: &KeyEvent,
        modifiers: Modifiers,
        pressed: bool,
        repeat: bool,
    ) -> KeyInput {
        KeyInput {
            keysym: event.keysym.raw(),
            raw_code: event.raw_code,
            text: if pressed { event.utf8.clone() } else { None },
            pressed,
            repeat,
            modifiers,
        }
    }

    fn push_key(&mut self, event: KeyEvent, pressed: bool, repeat: bool) {
        if let Some(surface) = self.keyboard_focus {
            let key = Self::convert_key(&event, self.modifiers, pressed, repeat);
            self.events.push_back(PlatformEvent::Key { surface, key });
        }
    }
}

fn convert_output_info(
    id: OutputId,
    info: &smithay_client_toolkit::output::OutputInfo,
) -> OutputInfo {
    let current = info.modes.iter().find(|m| m.current);
    let logical_size = info.logical_size.unwrap_or_else(|| {
        current
            .map(|m| {
                let s = info.scale_factor.max(1);
                (m.dimensions.0 / s, m.dimensions.1 / s)
            })
            .unwrap_or((0, 0))
    });
    OutputInfo {
        id,
        name: info.name.clone(),
        description: info.description.clone(),
        make: info.make.clone(),
        model: info.model.clone(),
        logical_position: info.logical_position.unwrap_or(info.location),
        logical_size,
        scale_factor: info.scale_factor,
        physical_size_mm: info.physical_size,
        refresh_mhz: current.map(|m| m.refresh_rate),
    }
}

fn convert_modifiers(m: &SctkModifiers) -> Modifiers {
    Modifiers {
        ctrl: m.ctrl,
        alt: m.alt,
        shift: m.shift,
        logo: m.logo,
        caps_lock: m.caps_lock,
        num_lock: m.num_lock,
    }
}

// ───────────────────────────── sctk handler implementations ─────────────────────────────

impl<E: 'static> CompositorHandler for Platform<E> {
    fn scale_factor_changed(
        &mut self,
        _: &Connection,
        _: &QueueHandle<Self>,
        surface: &wl_surface::WlSurface,
        new_factor: i32,
    ) {
        let Some(id) = self.surface_id(surface) else {
            return;
        };
        let mut changed = false;
        if let Some(entry) = self.surfaces.get_mut(&id) {
            if entry.int_scale != new_factor {
                entry.int_scale = new_factor;
                changed = entry.configured && entry.scale120.is_none();
            }
        }
        if changed {
            let scale = self.scale(id);
            self.events
                .push_back(PlatformEvent::ScaleChanged { surface: id, scale });
        }
    }

    fn transform_changed(
        &mut self,
        _: &Connection,
        _: &QueueHandle<Self>,
        _: &wl_surface::WlSurface,
        _: wl_output::Transform,
    ) {
    }

    fn frame(
        &mut self,
        _: &Connection,
        _: &QueueHandle<Self>,
        surface: &wl_surface::WlSurface,
        _: u32,
    ) {
        let Some(id) = self.surface_id(surface) else {
            return;
        };
        if let Some(entry) = self.surfaces.get_mut(&id) {
            entry.frame_pending = false;
        }
        self.events.push_back(PlatformEvent::Frame { surface: id });
    }

    fn surface_enter(
        &mut self,
        _: &Connection,
        _: &QueueHandle<Self>,
        _: &wl_surface::WlSurface,
        _: &wl_output::WlOutput,
    ) {
    }

    fn surface_leave(
        &mut self,
        _: &Connection,
        _: &QueueHandle<Self>,
        _: &wl_surface::WlSurface,
        _: &wl_output::WlOutput,
    ) {
    }
}

impl<E: 'static> OutputHandler for Platform<E> {
    fn output_state(&mut self) -> &mut OutputState {
        &mut self.output_state
    }

    fn new_output(&mut self, _: &Connection, _: &QueueHandle<Self>, output: wl_output::WlOutput) {
        let Some(info) = self.output_state.info(&output) else {
            return;
        };
        let id = OutputId(info.id);
        self.outputs.insert(id, output);
        let info = convert_output_info(id, &info);
        info!(name = ?info.name, size = ?info.logical_size, scale = info.scale_factor, "output added");
        self.events.push_back(PlatformEvent::OutputAdded(info));
    }

    fn update_output(
        &mut self,
        _: &Connection,
        _: &QueueHandle<Self>,
        output: wl_output::WlOutput,
    ) {
        let Some(info) = self.output_state.info(&output) else {
            return;
        };
        let id = OutputId(info.id);
        self.outputs.entry(id).or_insert(output);
        self.events
            .push_back(PlatformEvent::OutputChanged(convert_output_info(id, &info)));
    }

    fn output_destroyed(
        &mut self,
        _: &Connection,
        _: &QueueHandle<Self>,
        output: wl_output::WlOutput,
    ) {
        let id = self
            .outputs
            .iter()
            .find(|(_, o)| **o == output)
            .map(|(id, _)| *id)
            .or_else(|| self.output_state.info(&output).map(|i| OutputId(i.id)));
        if let Some(id) = id {
            self.outputs.remove(&id);
            info!(?id, "output removed");
            self.events.push_back(PlatformEvent::OutputRemoved(id));
        }
    }
}

impl<E: 'static> LayerShellHandler for Platform<E> {
    fn closed(&mut self, _: &Connection, _: &QueueHandle<Self>, layer: &LayerSurface) {
        if let Some(id) = self.surface_id(layer.wl_surface()) {
            self.events.push_back(PlatformEvent::Closed { surface: id });
        }
    }

    fn configure(
        &mut self,
        _: &Connection,
        _: &QueueHandle<Self>,
        layer: &LayerSurface,
        configure: LayerSurfaceConfigure,
        _: u32,
    ) {
        let Some(id) = self.surface_id(layer.wl_surface()) else {
            return;
        };
        let (mut width, mut height) = configure.new_size;
        {
            let entry = self.surfaces.get_mut(&id).expect("surface exists");
            if width == 0 {
                width = entry.logical_size.0.max(1);
            }
            if height == 0 {
                height = entry.logical_size.1.max(1);
            }
            entry.logical_size = (width, height);
            entry.configured = true;
            if let Some(viewport) = &entry.viewport {
                viewport.set_destination(width as i32, height as i32);
            }
        }
        let scale = self.scale(id);
        self.events.push_back(PlatformEvent::Configure {
            surface: id,
            width,
            height,
            scale,
        });
    }
}

impl<E: 'static> WindowHandler for Platform<E> {
    fn request_close(&mut self, _: &Connection, _: &QueueHandle<Self>, window: &Window) {
        if let Some(id) = self.surface_id(window.wl_surface()) {
            self.events.push_back(PlatformEvent::Closed { surface: id });
        }
    }

    fn configure(
        &mut self,
        _: &Connection,
        _: &QueueHandle<Self>,
        window: &Window,
        configure: WindowConfigure,
        _: u32,
    ) {
        let Some(id) = self.surface_id(window.wl_surface()) else {
            return;
        };
        {
            let entry = self.surfaces.get_mut(&id).expect("surface exists");
            let width = configure
                .new_size
                .0
                .map(|w| w.get())
                .unwrap_or(entry.logical_size.0.max(1));
            let height = configure
                .new_size
                .1
                .map(|h| h.get())
                .unwrap_or(entry.logical_size.1.max(1));
            entry.logical_size = (width, height);
            entry.configured = true;
            if let Some(viewport) = &entry.viewport {
                viewport.set_destination(width as i32, height as i32);
            }
        }
        let (width, height) = self.surfaces[&id].logical_size;
        let scale = self.scale(id);
        self.events.push_back(PlatformEvent::Configure {
            surface: id,
            width,
            height,
            scale,
        });
    }
}

impl<E: 'static> Platform<E> {
    /// Index of the entry for `seat`, creating it (with its clipboard data device) if needed.
    fn ensure_seat(&mut self, qh: &QueueHandle<Self>, seat: &wl_seat::WlSeat) -> usize {
        if let Some(i) = self.seats.iter().position(|s| &s.seat == seat) {
            return i;
        }
        let data_device = self
            .data_device_manager
            .as_ref()
            .map(|m| m.get_data_device(qh, seat));
        self.seats.push(SeatEntry {
            seat: seat.clone(),
            keyboard: None,
            pointer: None,
            data_device,
            last_serial: 0,
        });
        self.seats.len() - 1
    }
}

impl<E: 'static> SeatHandler for Platform<E> {
    fn seat_state(&mut self) -> &mut SeatState {
        &mut self.seat_state
    }

    fn new_seat(&mut self, _: &Connection, qh: &QueueHandle<Self>, seat: wl_seat::WlSeat) {
        self.ensure_seat(qh, &seat);
    }

    fn new_capability(
        &mut self,
        _: &Connection,
        qh: &QueueHandle<Self>,
        seat: wl_seat::WlSeat,
        capability: Capability,
    ) {
        let loop_handle = self.loop_handle.clone();
        // sctk only calls `new_seat` for seats announced after startup; seats that already
        // existed when we connected (the usual case) first show up here.
        let idx = self.ensure_seat(qh, &seat);
        let entry = &mut self.seats[idx];
        match capability {
            Capability::Keyboard if entry.keyboard.is_none() => {
                match self.seat_state.get_keyboard_with_repeat(
                    qh,
                    &seat,
                    None,
                    loop_handle,
                    Box::new(|state: &mut Self, _kbd, event| state.push_key(event, true, true)),
                ) {
                    Ok(kbd) => entry.keyboard = Some(kbd),
                    Err(e) => warn!("failed to create keyboard: {e}"),
                }
            }
            Capability::Pointer if entry.pointer.is_none() => {
                let surface = self.compositor.create_surface(qh);
                match self.seat_state.get_pointer_with_theme::<Self, ()>(
                    qh,
                    &seat,
                    self.shm.wl_shm(),
                    surface,
                    ThemeSpec::default(),
                ) {
                    Ok(p) => entry.pointer = Some(p),
                    Err(e) => warn!("failed to create pointer: {e}"),
                }
            }
            _ => {}
        }
    }

    fn remove_capability(
        &mut self,
        _: &Connection,
        _: &QueueHandle<Self>,
        seat: wl_seat::WlSeat,
        capability: Capability,
    ) {
        let Some(entry) = self.seats.iter_mut().find(|s| s.seat == seat) else {
            return;
        };
        match capability {
            Capability::Keyboard => {
                if let Some(k) = entry.keyboard.take() {
                    k.release();
                }
            }
            Capability::Pointer => {
                entry.pointer = None;
            }
            _ => {}
        }
    }

    fn remove_seat(&mut self, _: &Connection, _: &QueueHandle<Self>, seat: wl_seat::WlSeat) {
        self.seats.retain(|s| s.seat != seat);
    }
}

impl<E: 'static> KeyboardHandler for Platform<E> {
    fn enter(
        &mut self,
        _: &Connection,
        _: &QueueHandle<Self>,
        _: &wl_keyboard::WlKeyboard,
        surface: &wl_surface::WlSurface,
        serial: u32,
        _: &[u32],
        _: &[Keysym],
    ) {
        self.note_serial(serial);
        if let Some(id) = self.surface_id(surface) {
            self.keyboard_focus = Some(id);
            self.events
                .push_back(PlatformEvent::KeyboardEnter { surface: id });
        }
    }

    fn leave(
        &mut self,
        _: &Connection,
        _: &QueueHandle<Self>,
        _: &wl_keyboard::WlKeyboard,
        surface: &wl_surface::WlSurface,
        serial: u32,
    ) {
        self.note_serial(serial);
        if let Some(id) = self.surface_id(surface) {
            if self.keyboard_focus == Some(id) {
                self.keyboard_focus = None;
            }
            self.events
                .push_back(PlatformEvent::KeyboardLeave { surface: id });
        }
    }

    fn press_key(
        &mut self,
        _: &Connection,
        _: &QueueHandle<Self>,
        _: &wl_keyboard::WlKeyboard,
        serial: u32,
        event: KeyEvent,
    ) {
        self.note_serial(serial);
        self.push_key(event, true, false);
    }

    fn repeat_key(
        &mut self,
        _: &Connection,
        _: &QueueHandle<Self>,
        _: &wl_keyboard::WlKeyboard,
        _: u32,
        event: KeyEvent,
    ) {
        self.push_key(event, true, true);
    }

    fn release_key(
        &mut self,
        _: &Connection,
        _: &QueueHandle<Self>,
        _: &wl_keyboard::WlKeyboard,
        serial: u32,
        event: KeyEvent,
    ) {
        self.note_serial(serial);
        self.push_key(event, false, false);
    }

    fn update_modifiers(
        &mut self,
        _: &Connection,
        _: &QueueHandle<Self>,
        _: &wl_keyboard::WlKeyboard,
        _: u32,
        modifiers: SctkModifiers,
        _: RawModifiers,
        _: u32,
    ) {
        let m = convert_modifiers(&modifiers);
        if m != self.modifiers {
            self.modifiers = m;
            self.events
                .push_back(PlatformEvent::ModifiersChanged { modifiers: m });
        }
    }
}

impl<E: 'static> Platform<E> {
    fn note_serial(&mut self, serial: u32) {
        for seat in &mut self.seats {
            seat.last_serial = serial;
        }
    }
}

impl<E: 'static> PointerHandler for Platform<E> {
    fn pointer_frame(
        &mut self,
        _: &Connection,
        _: &QueueHandle<Self>,
        _: &wl_pointer::WlPointer,
        events: &[PointerEvent],
    ) {
        for event in events {
            let Some(id) = self.surface_id(&event.surface) else {
                continue;
            };
            let (x, y) = event.position;
            match event.kind {
                PointerEventKind::Enter { serial } => {
                    self.note_serial(serial);
                    self.pointer_focus = Some(id);
                    self.pointer_position = (x, y);
                    self.events
                        .push_back(PlatformEvent::PointerEnter { surface: id, x, y });
                }
                PointerEventKind::Leave { serial } => {
                    self.note_serial(serial);
                    if self.pointer_focus == Some(id) {
                        self.pointer_focus = None;
                    }
                    self.events
                        .push_back(PlatformEvent::PointerLeave { surface: id });
                }
                PointerEventKind::Motion { .. } => {
                    self.pointer_position = (x, y);
                    self.events
                        .push_back(PlatformEvent::PointerMotion { surface: id, x, y });
                }
                PointerEventKind::Press { button, serial, .. } => {
                    self.note_serial(serial);
                    self.events.push_back(PlatformEvent::PointerButton {
                        surface: id,
                        button,
                        pressed: true,
                        x,
                        y,
                    });
                }
                PointerEventKind::Release { button, serial, .. } => {
                    self.note_serial(serial);
                    self.events.push_back(PlatformEvent::PointerButton {
                        surface: id,
                        button,
                        pressed: false,
                        x,
                        y,
                    });
                }
                PointerEventKind::Axis {
                    horizontal,
                    vertical,
                    ..
                } => {
                    let dx = axis_value(&horizontal);
                    let dy = axis_value(&vertical);
                    if dx != 0.0 || dy != 0.0 {
                        self.events.push_back(PlatformEvent::PointerAxis {
                            surface: id,
                            dx,
                            dy,
                            x,
                            y,
                        });
                    }
                }
            }
        }
    }
}

fn axis_value(axis: &AxisScroll) -> f64 {
    scroll_amount(axis.absolute, axis.discrete)
}

/// Convert a wl_pointer axis event into logical pixels: absolute values win, discrete
/// steps count as 15 px each.
fn scroll_amount(absolute: f64, discrete: i32) -> f64 {
    if absolute != 0.0 {
        absolute
    } else if discrete != 0 {
        discrete as f64 * 15.0
    } else {
        0.0
    }
}

impl<E: 'static> ShmHandler for Platform<E> {
    fn shm_state(&mut self) -> &mut Shm {
        &mut self.shm
    }
}

impl<E: 'static> DataDeviceHandler for Platform<E> {
    fn enter(
        &mut self,
        _: &Connection,
        _: &QueueHandle<Self>,
        _: &WlDataDevice,
        _x: f64,
        _y: f64,
        _: &wl_surface::WlSurface,
    ) {
    }
    fn leave(&mut self, _: &Connection, _: &QueueHandle<Self>, _: &WlDataDevice) {}
    fn motion(
        &mut self,
        _: &Connection,
        _: &QueueHandle<Self>,
        _: &WlDataDevice,
        _x: f64,
        _y: f64,
    ) {
    }
    fn selection(&mut self, _: &Connection, _: &QueueHandle<Self>, _: &WlDataDevice) {}
    fn drop_performed(&mut self, _: &Connection, _: &QueueHandle<Self>, _: &WlDataDevice) {}
}

impl<E: 'static> DataOfferHandler for Platform<E> {
    fn source_actions(
        &mut self,
        _: &Connection,
        _: &QueueHandle<Self>,
        _: &mut DragOffer,
        _: DndAction,
    ) {
    }
    fn selected_action(
        &mut self,
        _: &Connection,
        _: &QueueHandle<Self>,
        _: &mut DragOffer,
        _: DndAction,
    ) {
    }
}

impl<E: 'static> DataSourceHandler for Platform<E> {
    fn accept_mime(
        &mut self,
        _: &Connection,
        _: &QueueHandle<Self>,
        _: &WlDataSource,
        _: Option<String>,
    ) {
    }

    fn send_request(
        &mut self,
        _: &Connection,
        _: &QueueHandle<Self>,
        source: &WlDataSource,
        _mime: String,
        mut fd: WritePipe,
    ) {
        if self
            .copy_source
            .as_ref()
            .map(|s| s.inner() == source)
            .unwrap_or(false)
        {
            let text = self.copy_text.clone();
            std::thread::spawn(move || {
                let _ = fd.write_all(text.as_bytes());
                let _ = fd.flush();
            });
        }
    }

    fn cancelled(&mut self, _: &Connection, _: &QueueHandle<Self>, source: &WlDataSource) {
        if self
            .copy_source
            .as_ref()
            .map(|s| s.inner() == source)
            .unwrap_or(false)
        {
            self.copy_source = None;
            self.copy_text.clear();
        }
    }

    fn dnd_dropped(&mut self, _: &Connection, _: &QueueHandle<Self>, _: &WlDataSource) {}
    fn dnd_finished(&mut self, _: &Connection, _: &QueueHandle<Self>, _: &WlDataSource) {}
    fn action(&mut self, _: &Connection, _: &QueueHandle<Self>, _: &WlDataSource, _: DndAction) {}
}

impl<E: 'static> SessionLockHandler for Platform<E> {
    fn locked(&mut self, _: &Connection, _: &QueueHandle<Self>, _: SessionLock) {
        info!("session locked");
    }

    fn finished(&mut self, _: &Connection, _: &QueueHandle<Self>, _: SessionLock) {
        // The compositor refused the lock (another client holds it) or ended it.
        warn!("session lock finished by the compositor");
        let ids: Vec<SurfaceId> = self
            .surfaces
            .iter()
            .filter(|(_, e)| e.role == SurfaceRole::Lock)
            .map(|(id, _)| *id)
            .collect();
        for id in ids {
            self.events.push_back(PlatformEvent::Closed { surface: id });
        }
    }

    fn configure(
        &mut self,
        _: &Connection,
        _: &QueueHandle<Self>,
        surface: SessionLockSurface,
        configure: SessionLockSurfaceConfigure,
        _: u32,
    ) {
        let Some(id) = self.surface_id(surface.wl_surface()) else {
            return;
        };
        {
            let entry = self.surfaces.get_mut(&id).expect("surface exists");
            let (w, h) = configure.new_size;
            entry.logical_size = (w.max(1), h.max(1));
            entry.configured = true;
            if let Some(viewport) = &entry.viewport {
                viewport.set_destination(w.max(1) as i32, h.max(1) as i32);
            }
        }
        let (width, height) = self.surfaces[&id].logical_size;
        let scale = self.scale(id);
        self.events.push_back(PlatformEvent::Configure {
            surface: id,
            width,
            height,
            scale,
        });
    }
}

delegate_registry!(@<E: 'static> Platform<E>);

impl<E: 'static> ProvidesRegistryState for Platform<E> {
    fn registry(&mut self) -> &mut RegistryState {
        &mut self.registry_state
    }
    registry_handlers![OutputState, SeatState];
}

smithay_client_toolkit::delegate_dispatch2!(@<E: 'static> Platform<E>);

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn scale_math_prefers_fractional() {
        let scale120 = Some(180u32);
        let scale = match scale120 {
            Some(s) if s > 0 => s as f64 / 120.0,
            _ => 2.0,
        };
        assert!((scale - 1.5).abs() < f64::EPSILON);
        let (w, h) = (
            (1920f64 * scale).round() as u32,
            (1080f64 * scale).round() as u32,
        );
        assert_eq!((w, h), (2880, 1620));
    }

    #[test]
    fn axis_prefers_absolute_then_discrete() {
        assert_eq!(scroll_amount(3.5, 0), 3.5);
        assert_eq!(scroll_amount(0.0, -2), -30.0);
        assert_eq!(scroll_amount(0.0, 0), 0.0);
    }
}
