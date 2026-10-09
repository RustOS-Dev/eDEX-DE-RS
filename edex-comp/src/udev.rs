use std::{
    collections::hash_map::HashMap,
    io,
    ops::Not,
    path::Path,
    sync::{atomic::Ordering, Mutex, Once},
    time::{Duration, Instant},
};

#[cfg(feature = "gpu")]
use crate::shell::WindowRenderElement;
use crate::{
    drawing::*,
    render::*,
    shell::WindowElement,
    state::{
        take_presentation_feedback, update_primary_scanout_output, Backend, EdexState, InitOptions,
    },
};
use crate::{
    dumb::{DumbError, DumbOutput},
    state::{DndIcon, SurfaceDmabufFeedback},
};
use smithay::backend::renderer::Color32F;
#[cfg(feature = "gpu")]
use smithay::backend::{
    allocator::{
        format::FormatSet,
        gbm::{GbmAllocator, GbmBufferFlags, GbmDevice},
        Modifier,
    },
    drm::{
        compositor::FrameFlags,
        exporter::gbm::GbmFramebufferExporter,
        output::{DrmOutput, DrmOutputManager, DrmOutputRenderElements},
        DrmSurface,
    },
    egl::{self, context::ContextPriority, EGLDevice, EGLDisplay},
    renderer::{
        damage::Error as OutputDamageTrackerError,
        gles::GlesRenderer,
        multigpu::{gbm::GbmGlesBackend, GpuManager, MultiRenderer},
        DebugFlags, ImportDma, ImportEgl,
    },
};
#[cfg(feature = "gpu")]
use smithay::reexports::drm::Device as _;
#[cfg(feature = "gpu")]
use smithay::reexports::wayland_protocols::wp::linux_dmabuf::zv1::server::zwp_linux_dmabuf_feedback_v1;
#[cfg(feature = "gpu")]
use smithay::wayland::{dmabuf::DmabufFeedbackBuilder, drm_syncobj::supports_syncobj_eventfd};
use smithay::{
    backend::{
        allocator::{dmabuf::Dmabuf, Fourcc},
        drm::{
            CreateDrmNodeError, DrmAccessError, DrmDevice, DrmDeviceFd, DrmError, DrmEvent,
            DrmEventMetadata, DrmEventTime, DrmNode, NodeType,
        },
        input::InputEvent,
        libinput::{LibinputInputBackend, LibinputSessionInterface},
        renderer::{
            element::{memory::MemoryRenderBuffer, AsRenderElements, RenderElementStates},
            pixman::PixmanRenderer,
            ImportAll, ImportMem, ImportMemWl, Renderer, Texture,
        },
        session::{
            libseat::{self, LibSeatSession},
            Event as SessionEvent, Session,
        },
        udev::{all_gpus, primary_gpu, UdevBackend, UdevEvent},
        SwapBuffersError,
    },
    delegate_dmabuf, delegate_drm_lease,
    desktop::{
        space::{Space, SurfaceTree},
        utils::OutputPresentationFeedback,
    },
    input::{
        keyboard::LedState,
        pointer::{CursorImageAttributes, CursorImageStatus},
    },
    output::{Mode as WlMode, Output, PhysicalProperties},
    reexports::{
        calloop::{
            timer::{TimeoutAction, Timer},
            EventLoop, RegistrationToken,
        },
        drm::control::{connector, crtc, Device, ModeTypeFlags},
        input::{DeviceCapability, Libinput},
        rustix::fs::OFlags,
        wayland_protocols::wp::presentation_time::server::wp_presentation_feedback,
        wayland_server::{backend::GlobalId, protocol::wl_surface, Display, DisplayHandle},
    },
    utils::{DeviceFd, IsAlive, Logical, Monotonic, Point, Rectangle, Scale, Time, Transform},
    wayland::{
        compositor,
        dmabuf::{DmabufGlobal, DmabufHandler, DmabufState, ImportNotifier},
        drm_lease::{
            DrmLease, DrmLeaseBuilder, DrmLeaseHandler, DrmLeaseRequest, DrmLeaseState,
            LeaseRejected,
        },
        drm_syncobj::{DrmSyncobjHandler, DrmSyncobjState},
        presentation::Refresh,
    },
};
use smithay_drm_extras::drm_scanner::{DrmScanEvent, DrmScanner};
use tracing::{debug, error, info, trace, warn};

// we cannot simply pick the first supported format of the intersection of *all* formats, because:
// - we do not want something like Abgr4444, which looses color information, if something better is available
// - some formats might perform terribly
// - we might need some work-arounds, if one supports modifiers, but the other does not
//
// So lets just pick `ARGB2101010` (10-bit) or `ARGB8888` (8-bit) for now, they are widely supported.
#[cfg(feature = "gpu")]
const SUPPORTED_FORMATS: &[Fourcc] = &[
    Fourcc::Abgr2101010,
    Fourcc::Argb2101010,
    Fourcc::Abgr8888,
    Fourcc::Argb8888,
];
#[cfg(feature = "gpu")]
const SUPPORTED_FORMATS_8BIT_ONLY: &[Fourcc] = &[Fourcc::Abgr8888, Fourcc::Argb8888];

#[cfg(feature = "gpu")]
type UdevRenderer<'a> = MultiRenderer<
    'a,
    'a,
    GbmGlesBackend<GlesRenderer, DrmDeviceFd>,
    GbmGlesBackend<GlesRenderer, DrmDeviceFd>,
>;

#[derive(Debug, PartialEq)]
struct UdevOutputId {
    device_id: DrmNode,
    crtc: crtc::Handle,
}

pub struct UdevData {
    pub session: LibSeatSession,
    dh: DisplayHandle,
    dmabuf_state: Option<(DmabufState, DmabufGlobal)>,
    syncobj_state: Option<DrmSyncobjState>,
    primary_gpu: DrmNode,
    #[cfg(feature = "gpu")]
    gpus: GpuManager<GbmGlesBackend<GlesRenderer, DrmDeviceFd>>,
    /// Software rendering into dumb buffers, for every output: built without `gpu`, asked for
    /// with `EDEX_RENDERER=pixman`, or GBM/EGL did not come up on the primary GPU.
    pixman: Option<PixmanRenderer>,
    backends: HashMap<DrmNode, BackendData>,
    pointer_images: Vec<(xcursor::parser::Image, MemoryRenderBuffer)>,
    pointer_element: PointerElement,
    pointer_image: crate::cursor::Cursor,
    #[cfg(feature = "gpu")]
    debug_flags: DebugFlags,
    keyboards: Vec<smithay::reexports::input::Device>,
    /// Every libinput device, for pointer settings.
    input_devices: Vec<smithay::reexports::input::Device>,
    dpms_on: bool,
    gamma: Option<u32>,
    /// Connectors switched off in the display settings.
    disabled_connectors: Vec<(DrmNode, connector::Info, crtc::Handle)>,
}

impl UdevData {
    #[cfg(feature = "gpu")]
    pub fn set_debug_flags(&mut self, flags: DebugFlags) {
        if self.debug_flags != flags {
            self.debug_flags = flags;

            for backend in self.backends.values_mut() {
                for surface in backend.surfaces.values_mut() {
                    if let OutputDrm::Gpu(drm_output) = &mut surface.drm_output {
                        drm_output.set_debug_flags(flags);
                    }
                }
            }
        }
    }

    #[cfg(feature = "gpu")]
    pub fn debug_flags(&self) -> DebugFlags {
        self.debug_flags
    }

    /// The renderer in use, for logs.
    pub fn renderer_name(&self) -> &'static str {
        if self.pixman.is_some() {
            "pixman"
        } else {
            "GLES"
        }
    }
}

impl DmabufHandler for EdexState<UdevData> {
    fn dmabuf_state(&mut self) -> &mut DmabufState {
        &mut self.backend_data.dmabuf_state.as_mut().unwrap().0
    }

    fn dmabuf_imported(
        &mut self,
        _global: &DmabufGlobal,
        dmabuf: Dmabuf,
        notifier: ImportNotifier,
    ) {
        #[cfg(feature = "gpu")]
        if self
            .backend_data
            .gpus
            .single_renderer(&self.backend_data.primary_gpu)
            .and_then(|mut renderer| renderer.import_dmabuf(&dmabuf, None))
            .is_ok()
        {
            dmabuf.set_node(self.backend_data.primary_gpu);
            let _ = notifier.successful::<EdexState<UdevData>>();
            return;
        }
        // The linux-dmabuf global only exists with GPU rendering.
        let _ = dmabuf;
        notifier.failed();
    }
}
delegate_dmabuf!(EdexState<UdevData>);

impl Backend for UdevData {
    const HAS_RELATIVE_MOTION: bool = true;
    const HAS_GESTURES: bool = true;

    fn seat_name(&self) -> String {
        self.session.seat()
    }

    fn reset_buffers(&mut self, output: &Output) {
        if let Some(id) = output.user_data().get::<UdevOutputId>() {
            if let Some(gpu) = self.backends.get_mut(&id.device_id) {
                if let Some(surface) = gpu.surfaces.get_mut(&id.crtc) {
                    surface.drm_output.reset_buffers();
                }
            }
        }
    }

    fn early_import(&mut self, surface: &wl_surface::WlSurface) {
        #[cfg(feature = "gpu")]
        if self.pixman.is_none() {
            if let Err(err) = self.gpus.early_import(self.primary_gpu, surface) {
                warn!("Early buffer import failed: {}", err);
            }
        }
        let _ = surface;
    }

    fn update_led_state(&mut self, led_state: LedState) {
        for keyboard in self.keyboards.iter_mut() {
            keyboard.led_update(led_state.into());
        }
    }

    fn set_gamma(state: &mut EdexState<Self>, kelvin: Option<u32>) {
        state.udev_set_gamma(kelvin);
    }

    fn set_dpms(state: &mut EdexState<Self>, on: bool) {
        state.udev_set_dpms(on);
    }

    fn apply_output_config(state: &mut EdexState<Self>) {
        state.udev_apply_output_config();
    }

    fn apply_input_config(state: &mut EdexState<Self>) {
        let settings = state.config.pointer;
        for device in state.backend_data.input_devices.iter_mut() {
            configure_pointer_device(device, &settings);
        }
    }

    fn screenshot(
        state: &mut EdexState<Self>,
        output: Option<&str>,
        region: Option<Rectangle<i32, Logical>>,
        path: &str,
    ) -> anyhow::Result<()> {
        state.udev_screenshot(output, region, path)
    }

    fn switch_vt(state: &mut EdexState<Self>, vt: i32) {
        info!(to = vt, "switching vt");
        if let Err(err) = state.backend_data.session.change_vt(vt) {
            error!(vt, "Error switching vt: {}", err);
        }
    }
}

pub fn run_udev(opts: InitOptions) {
    let mut event_loop = EventLoop::try_new().unwrap();
    let display = Display::new().unwrap();
    let mut display_handle = display.handle();

    /*
     * Initialize session
     */
    let (session, notifier) = match LibSeatSession::new() {
        Ok(ret) => ret,
        Err(err) => {
            error!("Could not initialize a session: {}", err);
            return;
        }
    };

    /*
     * Initialize the compositor
     */
    let primary_gpu = if let Ok(var) = std::env::var("EDEX_DRM_DEVICE") {
        DrmNode::from_path(var).expect("Invalid drm device path")
    } else {
        primary_gpu(session.seat())
            .unwrap()
            .and_then(|x| {
                DrmNode::from_path(x)
                    .ok()?
                    .node_with_type(NodeType::Render)?
                    .ok()
            })
            .unwrap_or_else(|| {
                all_gpus(session.seat())
                    .unwrap()
                    .into_iter()
                    .find_map(|x| DrmNode::from_path(x).ok())
                    .expect("No GPU!")
            })
    };
    info!("Using {} as primary gpu.", primary_gpu);

    #[cfg(feature = "gpu")]
    let gpus =
        GpuManager::new(GbmGlesBackend::with_context_priority(ContextPriority::High)).unwrap();
    // Software rendering when built without `gpu` or asked for; with `gpu` it is also the
    // fallback when GBM/EGL fail on the primary GPU (see `device_added`).
    let pixman =
        if cfg!(feature = "gpu") && std::env::var("EDEX_RENDERER").as_deref() != Ok("pixman") {
            None
        } else {
            match PixmanRenderer::new() {
                Ok(renderer) => Some(renderer),
                Err(err) => {
                    error!("Could not create the pixman renderer: {err}");
                    return;
                }
            }
        };

    let data = UdevData {
        dh: display_handle.clone(),
        dmabuf_state: None,
        syncobj_state: None,
        session,
        primary_gpu,
        #[cfg(feature = "gpu")]
        gpus,
        pixman,
        backends: HashMap::new(),
        pointer_image: crate::cursor::Cursor::load(),
        pointer_images: Vec::new(),
        pointer_element: PointerElement::default(),
        #[cfg(feature = "gpu")]
        debug_flags: DebugFlags::empty(),
        keyboards: Vec::new(),
        input_devices: Vec::new(),
        dpms_on: true,
        gamma: None,
        disabled_connectors: Vec::new(),
    };
    let greeter = opts.greeter;
    let mut state = EdexState::init(display, event_loop.handle(), data, true, opts);

    /*
     * Initialize the udev backend
     */
    let udev_backend = match UdevBackend::new(&state.seat_name) {
        Ok(ret) => ret,
        Err(err) => {
            error!(error = ?err, "Failed to initialize udev backend");
            return;
        }
    };

    /*
     * Initialize libinput backend
     */
    let mut libinput_context = Libinput::new_with_udev::<LibinputSessionInterface<LibSeatSession>>(
        state.backend_data.session.clone().into(),
    );
    libinput_context.udev_assign_seat(&state.seat_name).unwrap();
    let libinput_backend = LibinputInputBackend::new(libinput_context.clone());

    /*
     * Bind all our objects that get driven by the event loop
     */
    event_loop
        .handle()
        .insert_source(libinput_backend, move |mut event, _, data| {
            let dh = data.backend_data.dh.clone();
            if let InputEvent::DeviceAdded { device } = &mut event {
                configure_pointer_device(device, &data.config.pointer);
                data.backend_data.input_devices.push(device.clone());
                if device.has_capability(DeviceCapability::Keyboard) {
                    if let Some(led_state) = data
                        .seat
                        .get_keyboard()
                        .map(|keyboard| keyboard.led_state())
                    {
                        device.led_update(led_state.into());
                    }
                    data.backend_data.keyboards.push(device.clone());
                }
            } else if let InputEvent::DeviceRemoved { ref device } = event {
                data.backend_data.input_devices.retain(|d| d != device);
                if device.has_capability(DeviceCapability::Keyboard) {
                    data.backend_data.keyboards.retain(|item| item != device);
                }
            }

            data.process_input_event(&dh, event)
        })
        .unwrap();

    event_loop
        .handle()
        .insert_source(notifier, move |event, &mut (), data| match event {
            SessionEvent::PauseSession => {
                libinput_context.suspend();
                info!("pausing session");

                for backend in data.backend_data.backends.values_mut() {
                    backend.drm.pause();
                    backend.active_leases.clear();
                    if let Some(lease_global) = backend.leasing_global.as_mut() {
                        lease_global.suspend();
                    }
                }
            }
            SessionEvent::ActivateSession => {
                info!("resuming session");

                if let Err(err) = libinput_context.resume() {
                    error!("Failed to resume libinput context: {:?}", err);
                }
                for (node, backend) in data
                    .backend_data
                    .backends
                    .iter_mut()
                    .map(|(handle, backend)| (*handle, backend))
                {
                    backend.activate();
                    if let Some(lease_global) = backend.leasing_global.as_mut() {
                        lease_global.resume::<EdexState<UdevData>>();
                    }
                    data.handle
                        .insert_idle(move |data| data.render(node, None, data.clock.now()));
                }
            }
        })
        .unwrap();

    // We try to initialize the primary node before others to make sure
    // any display only node can fall back to the primary node for rendering
    let primary_node = primary_gpu
        .node_with_type(NodeType::Primary)
        .and_then(|node| node.ok());
    let primary_device = udev_backend.device_list().find(|(device_id, _)| {
        primary_node
            .map(|primary_node| *device_id == primary_node.dev_id())
            .unwrap_or(false)
            || *device_id == primary_gpu.dev_id()
    });

    if let Some((device_id, path)) = primary_device {
        let node = DrmNode::from_dev_id(device_id).expect("failed to get primary node");
        state
            .device_added(node, path)
            .expect("failed to initialize primary node");
    }

    let primary_device_id = primary_device.map(|(device_id, _)| device_id);
    for (device_id, path) in udev_backend.device_list() {
        if Some(device_id) == primary_device_id {
            continue;
        }

        if let Err(err) = DrmNode::from_dev_id(device_id)
            .map_err(DeviceAddError::DrmNode)
            .and_then(|node| state.device_added(node, path))
        {
            error!("Skipping device {device_id}: {err}");
        }
    }
    let shm_formats = match state.backend_data.pixman.as_ref() {
        Some(renderer) => renderer.shm_formats().collect::<Vec<_>>(),
        #[cfg(feature = "gpu")]
        None => state
            .backend_data
            .gpus
            .single_renderer(&primary_gpu)
            .unwrap()
            .shm_formats()
            .collect(),
        #[cfg(not(feature = "gpu"))]
        None => unreachable!("built without GPU rendering"),
    };
    state.shm_state.update_formats(shm_formats);
    info!(
        renderer = state.backend_data.renderer_name(),
        "rendering with {}",
        state.backend_data.renderer_name()
    );

    // linux-dmabuf, wl_drm and explicit sync need the GPU renderer; pixman takes shm only.
    #[cfg(feature = "gpu")]
    if state.backend_data.pixman.is_none() {
        init_gpu_globals(&mut state, &display_handle, primary_gpu);
    }

    event_loop
        .handle()
        .insert_source(udev_backend, move |event, _, data| match event {
            UdevEvent::Added { device_id, path } => {
                if let Err(err) = DrmNode::from_dev_id(device_id)
                    .map_err(DeviceAddError::DrmNode)
                    .and_then(|node| data.device_added(node, &path))
                {
                    error!("Skipping device {device_id}: {err}");
                }
            }
            UdevEvent::Changed { device_id } => {
                if let Ok(node) = DrmNode::from_dev_id(device_id) {
                    data.device_changed(node)
                }
            }
            UdevEvent::Removed { device_id } => {
                if let Ok(node) = DrmNode::from_dev_id(device_id) {
                    data.device_removed(node)
                }
            }
        })
        .unwrap();

    state.apply_config();
    if greeter {
        state.start_greeter();
    } else {
        state.start_direct_session();
    }

    /*
     * And run our loop
     */

    while state.running.load(Ordering::SeqCst) {
        let result = event_loop.dispatch(Some(Duration::from_millis(16)), &mut state);
        if result.is_err() {
            state.running.store(false, Ordering::SeqCst);
        } else {
            state.handle_control();
            state.flush();
            state.space.refresh();
            state.popups.cleanup();
            display_handle.flush_clients().unwrap();
        }
    }
}

/// wl_drm (EGL), linux-dmabuf with per-surface feedback, and explicit sync when the primary GPU
/// supports it.
#[cfg(feature = "gpu")]
fn init_gpu_globals(
    state: &mut EdexState<UdevData>,
    display_handle: &DisplayHandle,
    primary_gpu: DrmNode,
) {
    let mut renderer = state
        .backend_data
        .gpus
        .single_renderer(&primary_gpu)
        .unwrap();

    info!(
        ?primary_gpu,
        "Trying to initialize EGL Hardware Acceleration",
    );
    match renderer.bind_wl_display(display_handle) {
        Ok(_) => info!("EGL hardware-acceleration enabled"),
        Err(err) => info!(?err, "Failed to initialize EGL hardware-acceleration"),
    }

    // init dmabuf support with format list from our primary gpu
    let dmabuf_formats = renderer.dmabuf_formats();
    let default_feedback = DmabufFeedbackBuilder::new(primary_gpu.dev_id(), dmabuf_formats)
        .build()
        .unwrap();
    let mut dmabuf_state = DmabufState::new();
    let global = dmabuf_state.create_global_with_default_feedback::<EdexState<UdevData>>(
        display_handle,
        &default_feedback,
    );
    state.backend_data.dmabuf_state = Some((dmabuf_state, global));

    let gpus = &mut state.backend_data.gpus;
    state
        .backend_data
        .backends
        .iter_mut()
        .for_each(|(node, backend_data)| {
            // Update the per drm surface dmabuf feedback
            backend_data.surfaces.values_mut().for_each(|surface_data| {
                let OutputDrm::Gpu(drm_output) = &mut surface_data.drm_output else {
                    return;
                };
                surface_data.dmabuf_feedback = surface_data.dmabuf_feedback.take().or_else(|| {
                    drm_output.with_compositor(|compositor| {
                        get_surface_dmabuf_feedback(
                            primary_gpu,
                            surface_data.render_node,
                            *node,
                            gpus,
                            compositor.surface(),
                        )
                    })
                });
            });
        });

    // Expose syncobj protocol if supported by primary GPU
    if let Some(primary_node) = state
        .backend_data
        .primary_gpu
        .node_with_type(NodeType::Primary)
        .and_then(|x| x.ok())
    {
        if let Some(backend) = state.backend_data.backends.get(&primary_node) {
            let import_device = backend.drm.device().device_fd().clone();
            if supports_syncobj_eventfd(&import_device) {
                let syncobj_state =
                    DrmSyncobjState::new::<EdexState<UdevData>>(display_handle, import_device);
                state.backend_data.syncobj_state = Some(syncobj_state);
            }
        }
    }
}

/// Tap-to-click, natural scrolling and pointer speed for a libinput device.
fn configure_pointer_device(
    device: &mut smithay::reexports::input::Device,
    settings: &crate::config::PointerSettings,
) {
    if device.config_tap_finger_count() > 0 {
        let _ = device.config_tap_set_enabled(settings.tap_to_click);
    }
    if device.config_scroll_has_natural_scroll() {
        let _ = device.config_scroll_set_natural_scroll_enabled(settings.natural_scroll);
    }
    if device.config_accel_is_available() {
        let _ = device.config_accel_set_speed(settings.accel_speed);
    }
}

impl DrmLeaseHandler for EdexState<UdevData> {
    fn drm_lease_state(&mut self, node: DrmNode) -> &mut DrmLeaseState {
        self.backend_data
            .backends
            .get_mut(&node)
            .unwrap()
            .leasing_global
            .as_mut()
            .unwrap()
    }

    fn lease_request(
        &mut self,
        node: DrmNode,
        request: DrmLeaseRequest,
    ) -> Result<DrmLeaseBuilder, LeaseRejected> {
        let backend = self
            .backend_data
            .backends
            .get(&node)
            .ok_or(LeaseRejected::default())?;

        let drm_device = backend.drm.device();
        let mut builder = DrmLeaseBuilder::new(drm_device);
        for conn in request.connectors {
            if let Some((_, crtc)) = backend
                .non_desktop_connectors
                .iter()
                .find(|(handle, _)| *handle == conn)
            {
                builder.add_connector(conn);
                builder.add_crtc(*crtc);
                let planes = drm_device.planes(crtc).map_err(LeaseRejected::with_cause)?;
                let (primary_plane, primary_plane_claim) = planes
                    .primary
                    .iter()
                    .find_map(|plane| {
                        drm_device
                            .claim_plane(plane.handle, *crtc)
                            .map(|claim| (plane, claim))
                    })
                    .ok_or_else(LeaseRejected::default)?;
                builder.add_plane(primary_plane.handle, primary_plane_claim);
                if let Some((cursor, claim)) = planes.cursor.iter().find_map(|plane| {
                    drm_device
                        .claim_plane(plane.handle, *crtc)
                        .map(|claim| (plane, claim))
                }) {
                    builder.add_plane(cursor.handle, claim);
                }
            } else {
                tracing::warn!(
                    ?conn,
                    "Lease requested for desktop connector, denying request"
                );
                return Err(LeaseRejected::default());
            }
        }

        Ok(builder)
    }

    fn new_active_lease(&mut self, node: DrmNode, lease: DrmLease) {
        let backend = self.backend_data.backends.get_mut(&node).unwrap();
        backend.active_leases.push(lease);
    }

    fn lease_destroyed(&mut self, node: DrmNode, lease: u32) {
        let backend = self.backend_data.backends.get_mut(&node).unwrap();
        backend.active_leases.retain(|l| l.id() != lease);
    }
}

delegate_drm_lease!(EdexState<UdevData>);

impl DrmSyncobjHandler for EdexState<UdevData> {
    fn drm_syncobj_state(&mut self) -> Option<&mut DrmSyncobjState> {
        self.backend_data.syncobj_state.as_mut()
    }
}
smithay::delegate_drm_syncobj!(EdexState<UdevData>);

#[cfg(feature = "gpu")]
type GbmDrmOutputManager = DrmOutputManager<
    GbmAllocator<DrmDeviceFd>,
    GbmFramebufferExporter<DrmDeviceFd>,
    Option<OutputPresentationFeedback>,
    DrmDeviceFd,
>;

#[cfg(feature = "gpu")]
type GbmDrmOutput = DrmOutput<
    GbmAllocator<DrmDeviceFd>,
    GbmFramebufferExporter<DrmDeviceFd>,
    Option<OutputPresentationFeedback>,
    DrmDeviceFd,
>;

/// How a CRTC is driven: Smithay's `DrmOutput` (GBM buffers, GLES, plane scan-out) or a
/// pixman-rendered dumb-buffer swapchain. One per output: the size difference does not matter.
#[allow(clippy::large_enum_variant)]
enum OutputDrm {
    #[cfg(feature = "gpu")]
    Gpu(GbmDrmOutput),
    Pixman(DumbOutput<Option<OutputPresentationFeedback>>),
}

impl OutputDrm {
    /// Forget what the buffers hold: the next frame is drawn in full. Only the ages are reset;
    /// dropping the GPU swapchain's buffers would remove the framebuffer on screen, and the
    /// kernel turns a CRTC off when its framebuffer is removed (RMFB).
    fn reset_buffers(&mut self) {
        match self {
            #[cfg(feature = "gpu")]
            OutputDrm::Gpu(o) => o.with_compositor(|c| c.reset_buffer_ages()),
            OutputDrm::Pixman(o) => o.reset_buffers(),
        }
    }

    /// The queued frame reached the screen; returns its presentation feedback.
    fn frame_submitted(
        &mut self,
    ) -> Result<Option<Option<OutputPresentationFeedback>>, SwapBuffersError> {
        match self {
            #[cfg(feature = "gpu")]
            OutputDrm::Gpu(o) => o.frame_submitted().map_err(Into::<SwapBuffersError>::into),
            OutputDrm::Pixman(o) => Ok(o.frame_submitted()),
        }
    }

    /// DPMS off: disable the CRTC until the next frame.
    fn clear(&mut self) -> Result<(), DrmError> {
        match self {
            #[cfg(feature = "gpu")]
            OutputDrm::Gpu(o) => o.with_compositor(|c| c.clear()),
            OutputDrm::Pixman(o) => o.clear(),
        }
    }
}

/// A DRM device: its outputs share either Smithay's output manager (GPU) or the bare device
/// (pixman).
#[allow(clippy::large_enum_variant)]
enum DeviceDrm {
    #[cfg(feature = "gpu")]
    Gpu(GbmDrmOutputManager),
    Pixman(DrmDevice),
}

impl DeviceDrm {
    fn device(&self) -> &DrmDevice {
        match self {
            #[cfg(feature = "gpu")]
            DeviceDrm::Gpu(m) => m.device(),
            DeviceDrm::Pixman(d) => d,
        }
    }

    fn device_mut(&mut self) -> &mut DrmDevice {
        match self {
            #[cfg(feature = "gpu")]
            DeviceDrm::Gpu(m) => m.device_mut(),
            DeviceDrm::Pixman(d) => d,
        }
    }

    fn pause(&mut self) {
        match self {
            #[cfg(feature = "gpu")]
            DeviceDrm::Gpu(m) => m.pause(),
            DeviceDrm::Pixman(d) => d.pause(),
        }
    }
}

struct SurfaceData {
    dh: DisplayHandle,
    device_id: DrmNode,
    render_node: Option<DrmNode>,
    global: Option<GlobalId>,
    connector: connector::Info,
    drm_output: OutputDrm,
    #[cfg(feature = "gpu")]
    disable_direct_scanout: bool,
    dmabuf_feedback: Option<SurfaceDmabufFeedback>,
    last_presentation_time: Option<Time<Monotonic>>,
    vblank_throttle_timer: Option<RegistrationToken>,
}

impl Drop for SurfaceData {
    fn drop(&mut self) {
        if let Some(global) = self.global.take() {
            self.dh.remove_global::<EdexState<UdevData>>(global);
        }
    }
}

struct BackendData {
    surfaces: HashMap<crtc::Handle, SurfaceData>,
    non_desktop_connectors: Vec<(connector::Handle, crtc::Handle)>,
    leasing_global: Option<DrmLeaseState>,
    active_leases: Vec<DrmLease>,
    drm: DeviceDrm,
    drm_scanner: DrmScanner,
    render_node: Option<DrmNode>,
    registration_token: RegistrationToken,
}

impl BackendData {
    /// The session is active again (VT switch back): re-read the CRTC state.
    fn activate(&mut self) {
        // if we do not care about flicking (caused by modesetting) we could just pass true for
        // disable connectors here. this would make sure our drm device is in a known state (all
        // connectors and planes disabled). but we choose a more optimistic path by leaving the
        // state as is and assume it will just work. If this assumption fails we will try to
        // reset the state when trying to queue a frame.
        match &mut self.drm {
            #[cfg(feature = "gpu")]
            DeviceDrm::Gpu(m) => m.activate(false).expect("failed to activate drm backend"),
            DeviceDrm::Pixman(d) => {
                d.activate(false).expect("failed to activate drm backend");
                for surface in self.surfaces.values_mut() {
                    #[allow(irrefutable_let_patterns)]
                    if let OutputDrm::Pixman(o) = &mut surface.drm_output {
                        if let Err(err) = o.reset_state() {
                            warn!("resetting the CRTC state: {err}");
                        }
                    }
                }
            }
        }
    }
}

#[derive(Debug, thiserror::Error)]
enum DeviceAddError {
    #[error("Failed to open device using libseat: {0}")]
    DeviceOpen(libseat::Error),
    #[error("Failed to initialize drm device: {0}")]
    DrmDevice(DrmError),
    #[cfg(feature = "gpu")]
    #[error("Failed to initialize gbm device: {0}")]
    GbmDevice(std::io::Error),
    #[error("Failed to access drm node: {0}")]
    DrmNode(CreateDrmNodeError),
    #[cfg(feature = "gpu")]
    #[error("Failed to add device to GpuManager: {0}")]
    AddNode(egl::Error),
    #[cfg(feature = "gpu")]
    #[error("Primary GPU is missing")]
    PrimaryGpuMissing,
    #[cfg(feature = "gpu")]
    #[error("Could not create the pixman renderer: {0}")]
    Pixman(smithay::backend::renderer::pixman::PixmanError),
}

#[cfg(feature = "gpu")]
fn get_surface_dmabuf_feedback(
    primary_gpu: DrmNode,
    render_node: Option<DrmNode>,
    scanout_node: DrmNode,
    gpus: &mut GpuManager<GbmGlesBackend<GlesRenderer, DrmDeviceFd>>,
    surface: &DrmSurface,
) -> Option<SurfaceDmabufFeedback> {
    let primary_formats = gpus.single_renderer(&primary_gpu).ok()?.dmabuf_formats();
    let render_formats = if let Some(render_node) = render_node {
        gpus.single_renderer(&render_node).ok()?.dmabuf_formats()
    } else {
        FormatSet::default()
    };

    let all_render_formats = primary_formats
        .iter()
        .chain(render_formats.iter())
        .copied()
        .collect::<FormatSet>();

    let planes = surface.planes().clone();

    // We limit the scan-out tranche to formats we can also render from
    // so that there is always a fallback render path available in case
    // the supplied buffer can not be scanned out directly
    let planes_formats = surface
        .plane_info()
        .formats
        .iter()
        .copied()
        .chain(planes.overlay.into_iter().flat_map(|p| p.formats))
        .collect::<FormatSet>()
        .intersection(&all_render_formats)
        .copied()
        .collect::<FormatSet>();

    let builder = DmabufFeedbackBuilder::new(primary_gpu.dev_id(), primary_formats);
    let render_feedback = if let Some(render_node) = render_node {
        builder
            .clone()
            .add_preference_tranche(render_node.dev_id(), None, render_formats.clone())
            .build()
            .unwrap()
    } else {
        builder.clone().build().unwrap()
    };

    let scanout_feedback = builder
        .add_preference_tranche(
            surface.device_fd().dev_id().unwrap(),
            Some(zwp_linux_dmabuf_feedback_v1::TrancheFlags::Scanout),
            planes_formats,
        )
        .add_preference_tranche(scanout_node.dev_id(), None, render_formats)
        .build()
        .unwrap();

    Some(SurfaceDmabufFeedback {
        render_feedback,
        scanout_feedback,
    })
}

impl EdexState<UdevData> {
    fn device_added(&mut self, node: DrmNode, path: &Path) -> Result<(), DeviceAddError> {
        // Try to open the device
        let fd = self
            .backend_data
            .session
            .open(
                path,
                OFlags::RDWR | OFlags::CLOEXEC | OFlags::NOCTTY | OFlags::NONBLOCK,
            )
            .map_err(DeviceAddError::DeviceOpen)?;

        let fd = DrmDeviceFd::new(DeviceFd::from(fd));

        let (drm, notifier) =
            DrmDevice::new(fd.clone(), true).map_err(DeviceAddError::DrmDevice)?;

        #[cfg(feature = "gpu")]
        let (drm, render_node) = match self.gpu_device(node, &fd, drm)? {
            Ok(gpu) => gpu,
            Err(drm) => (DeviceDrm::Pixman(drm), None),
        };
        #[cfg(not(feature = "gpu"))]
        let (drm, render_node) = (DeviceDrm::Pixman(drm), None);
        if matches!(drm, DeviceDrm::Pixman(_)) {
            info!("{node}: software rendering (pixman, dumb buffers)");
        }

        let registration_token = self
            .handle
            .insert_source(
                notifier,
                move |event, metadata, data: &mut EdexState<_>| match event {
                    DrmEvent::VBlank(crtc) => {
                        data.frame_finish(node, crtc, metadata);
                    }
                    DrmEvent::Error(error) => {
                        error!("{:?}", error);
                    }
                },
            )
            .unwrap();

        self.backend_data.backends.insert(
            node,
            BackendData {
                registration_token,
                drm,
                drm_scanner: DrmScanner::new(),
                non_desktop_connectors: Vec::new(),
                render_node,
                surfaces: HashMap::new(),
                leasing_global: DrmLeaseState::new::<EdexState<UdevData>>(
                    &self.display_handle,
                    &node,
                )
                .inspect_err(|err| {
                    warn!(?err, "Failed to initialize drm lease global for: {}", node);
                })
                .ok(),
                active_leases: Vec::new(),
            },
        );

        self.device_changed(node);

        Ok(())
    }

    /// Set a device up for GBM/EGL rendering. `Ok(Err(drm))` hands the device back for pixman:
    /// software rendering is on, or GBM/EGL do not work on the first device (no libEGL, no
    /// driver), which switches every output to pixman.
    #[cfg(feature = "gpu")]
    fn gpu_device(
        &mut self,
        node: DrmNode,
        fd: &DrmDeviceFd,
        drm: DrmDevice,
    ) -> Result<Result<(DeviceDrm, Option<DrmNode>), DrmDevice>, DeviceAddError> {
        if self.backend_data.pixman.is_some() {
            return Ok(Err(drm));
        }
        let gbm = match GbmDevice::new(fd.clone()) {
            Ok(gbm) => gbm,
            Err(err) if self.backend_data.backends.is_empty() => {
                warn!(?err, "no GBM device: falling back to pixman");
                self.backend_data.pixman =
                    Some(PixmanRenderer::new().map_err(DeviceAddError::Pixman)?);
                return Ok(Err(drm));
            }
            Err(err) => return Err(DeviceAddError::GbmDevice(err)),
        };

        let mut try_initialize_gpu = || {
            let display = unsafe { EGLDisplay::new(gbm.clone()).map_err(DeviceAddError::AddNode)? };
            let egl_device =
                EGLDevice::device_for_display(&display).map_err(DeviceAddError::AddNode)?;

            // A software EGL device (Mesa's llvmpipe/softpipe through kms_swrast) is fine:
            // it renders into GBM buffers like a GPU.
            let render_node = egl_device
                .try_get_render_node()
                .ok()
                .flatten()
                .unwrap_or(node);
            self.backend_data
                .gpus
                .as_mut()
                .add_node(render_node, gbm.clone())
                .map_err(DeviceAddError::AddNode)?;

            std::result::Result::<DrmNode, DeviceAddError>::Ok(render_node)
        };

        let render_node = try_initialize_gpu()
            .inspect_err(|err| {
                warn!(?err, "failed to initialize gpu");
            })
            .ok();

        if render_node.is_none() && self.backend_data.backends.is_empty() {
            warn!("GBM/EGL rendering is unavailable on {node}: falling back to pixman");
            self.backend_data.pixman = Some(PixmanRenderer::new().map_err(DeviceAddError::Pixman)?);
            return Ok(Err(drm));
        }

        let allocator = render_node
            .is_some()
            .then(|| {
                GbmAllocator::new(
                    gbm.clone(),
                    GbmBufferFlags::RENDERING | GbmBufferFlags::SCANOUT,
                )
            })
            .or_else(|| {
                self.backend_data
                    .backends
                    .get(&self.backend_data.primary_gpu)
                    .or_else(|| {
                        self.backend_data.backends.values().find(|backend| {
                            backend.render_node == Some(self.backend_data.primary_gpu)
                        })
                    })
                    .and_then(|backend| match &backend.drm {
                        DeviceDrm::Gpu(m) => Some(m.allocator().clone()),
                        DeviceDrm::Pixman(_) => None,
                    })
            })
            .ok_or(DeviceAddError::PrimaryGpuMissing)?;

        let framebuffer_exporter = GbmFramebufferExporter::new(gbm.clone(), render_node);

        let color_formats = if std::env::var("ANVIL_DISABLE_10BIT").is_ok() {
            SUPPORTED_FORMATS_8BIT_ONLY
        } else {
            SUPPORTED_FORMATS
        };
        let mut renderer = self
            .backend_data
            .gpus
            .single_renderer(&render_node.unwrap_or(self.backend_data.primary_gpu))
            .unwrap();
        let render_formats = renderer
            .as_mut()
            .egl_context()
            .dmabuf_render_formats()
            .iter()
            .filter(|format| render_node.is_some() || format.modifier == Modifier::Linear)
            .copied()
            .collect::<FormatSet>();

        let drm_output_manager = DrmOutputManager::new(
            drm,
            allocator,
            framebuffer_exporter,
            Some(gbm),
            color_formats.iter().copied(),
            render_formats,
        );
        Ok(Ok((DeviceDrm::Gpu(drm_output_manager), render_node)))
    }

    fn connector_connected(
        &mut self,
        node: DrmNode,
        connector: connector::Info,
        crtc: crtc::Handle,
    ) {
        let renderer_name = self.backend_data.renderer_name();
        let device = if let Some(device) = self.backend_data.backends.get_mut(&node) {
            device
        } else {
            return;
        };

        let output_name = format!(
            "{}-{}",
            connector.interface().as_str(),
            connector.interface_id()
        );
        info!(?crtc, "Trying to setup connector {}", output_name,);

        let drm_device = device.drm.device();

        let non_desktop = drm_device
            .get_properties(connector.handle())
            .ok()
            .and_then(|props| {
                let (info, value) = props
                    .into_iter()
                    .filter_map(|(handle, value)| {
                        let info = drm_device.get_property(handle).ok()?;

                        Some((info, value))
                    })
                    .find(|(info, _)| info.name().to_str() == Ok("non-desktop"))?;

                info.value_type().convert_value(value).as_boolean()
            })
            .unwrap_or(false);

        let edid = crate::edid::for_connector(drm_device, connector.handle()).unwrap_or_default();
        let make = if edid.make.is_empty() {
            "Unknown".to_string()
        } else {
            edid.make.clone()
        };
        let model = if edid.model.is_empty() {
            "Unknown".to_string()
        } else {
            edid.model.clone()
        };

        if non_desktop {
            info!(
                "Connector {} is non-desktop, setting up for leasing",
                output_name
            );
            device
                .non_desktop_connectors
                .push((connector.handle(), crtc));
            if let Some(lease_state) = device.leasing_global.as_mut() {
                lease_state.add_connector::<EdexState<UdevData>>(
                    connector.handle(),
                    output_name,
                    format!("{} {}", make, model),
                );
            }
        } else {
            let rule = self.config.output_rule(&output_name);
            if rule.disabled {
                info!("{output_name} is disabled in the display settings");
                self.backend_data
                    .disabled_connectors
                    .push((node, connector, crtc));
                return;
            }
            let Some(drm_mode) = pick_mode(&connector, rule.mode) else {
                warn!("{output_name} has no modes");
                return;
            };
            let wl_mode = WlMode::from(drm_mode);
            let preferred = connector
                .modes()
                .iter()
                .find(|mode| mode.mode_type().contains(ModeTypeFlags::PREFERRED))
                .copied()
                .unwrap_or(drm_mode);

            let (phys_w, phys_h) = connector.size().unwrap_or((0, 0));
            let output = Output::new(
                output_name,
                PhysicalProperties {
                    size: (phys_w as i32, phys_h as i32).into(),
                    subpixel: connector.subpixel().into(),
                    make,
                    model,
                },
            );
            let global = output.create_global::<EdexState<UdevData>>(&self.display_handle);

            let x = self.space.outputs().fold(0, |acc, o| {
                acc + self.space.output_geometry(o).map(|g| g.size.w).unwrap_or(0)
            });
            let position = rule.position.unwrap_or((x, 0)).into();

            for mode in connector.modes() {
                output.add_mode(WlMode::from(*mode));
            }
            output.set_preferred(WlMode::from(preferred));
            output.change_current_state(
                Some(wl_mode),
                Some(crate::manage::transform_from_index(rule.transform)),
                Some(smithay::output::Scale::Fractional(rule.scale)),
                Some(position),
            );
            self.space.map_output(&output, position);

            output.user_data().insert_if_missing(|| UdevOutputId {
                crtc,
                device_id: node,
            });

            let (w, h) = drm_mode.size();
            let drm_output = match &mut device.drm {
                #[cfg(feature = "gpu")]
                DeviceDrm::Gpu(manager) => {
                    let render_node = device.render_node.unwrap_or(self.backend_data.primary_gpu);
                    let mut renderer = self
                        .backend_data
                        .gpus
                        .single_renderer(&render_node)
                        .unwrap();
                    init_gpu_output(manager, crtc, drm_mode, &connector, &output, &mut renderer)
                        .map(OutputDrm::Gpu)
                }
                DeviceDrm::Pixman(drm) => drm
                    .create_surface(crtc, drm_mode, &[connector.handle()])
                    .map(|surface| {
                        OutputDrm::Pixman(DumbOutput::new(
                            surface,
                            drm.device_fd().clone(),
                            &output,
                        ))
                    })
                    .map_err(|e| e.to_string()),
            };
            let mut drm_output = match drm_output {
                Ok(drm_output) => drm_output,
                Err(err) => {
                    warn!("Failed to initialize drm output: {}", err);
                    self.space.unmap_output(&output);
                    self.display_handle
                        .remove_global::<EdexState<UdevData>>(global);
                    return;
                }
            };

            #[cfg(feature = "gpu")]
            let disable_direct_scanout = std::env::var("ANVIL_DISABLE_DIRECT_SCANOUT").is_ok();

            let dmabuf_feedback = match &mut drm_output {
                #[cfg(feature = "gpu")]
                OutputDrm::Gpu(drm_output) => drm_output.with_compositor(|compositor| {
                    compositor.set_debug_flags(self.backend_data.debug_flags);

                    get_surface_dmabuf_feedback(
                        self.backend_data.primary_gpu,
                        device.render_node,
                        node,
                        &mut self.backend_data.gpus,
                        compositor.surface(),
                    )
                }),
                OutputDrm::Pixman(_) => None,
            };

            let surface = SurfaceData {
                dh: self.display_handle.clone(),
                device_id: node,
                render_node: device.render_node,
                global: Some(global),
                connector: connector.clone(),
                drm_output,
                #[cfg(feature = "gpu")]
                disable_direct_scanout,
                dmabuf_feedback,
                last_presentation_time: None,
                vblank_throttle_timer: None,
            };

            info!(
                "output {} enabled {}x{} ({})",
                output.name(),
                w,
                h,
                renderer_name
            );
            device.surfaces.insert(crtc, surface);
            self.wm_output_added(&output);
            if let Some(kelvin) = self.backend_data.gamma {
                set_crtc_gamma(
                    self.backend_data
                        .backends
                        .get(&node)
                        .map(|b| b.drm.device()),
                    crtc,
                    Some(kelvin),
                );
            }

            // kick-off rendering
            self.handle.insert_idle(move |state| {
                state.render_surface(node, crtc, state.clock.now());
            });
        }
    }

    fn connector_disconnected(
        &mut self,
        node: DrmNode,
        connector: connector::Info,
        crtc: crtc::Handle,
    ) {
        let device = if let Some(device) = self.backend_data.backends.get_mut(&node) {
            device
        } else {
            return;
        };

        if let Some(pos) = device
            .non_desktop_connectors
            .iter()
            .position(|(handle, _)| *handle == connector.handle())
        {
            let _ = device.non_desktop_connectors.remove(pos);
            if let Some(leasing_state) = device.leasing_global.as_mut() {
                leasing_state.withdraw_connector(connector.handle());
            }
        } else {
            device.surfaces.remove(&crtc);

            let output = self
                .space
                .outputs()
                .find(|o| {
                    o.user_data()
                        .get::<UdevOutputId>()
                        .map(|id| id.device_id == node && id.crtc == crtc)
                        .unwrap_or(false)
                })
                .cloned();

            if let Some(output) = output {
                self.space.unmap_output(&output);
                self.wm.remove_output(&output.name());
                self.layout_dirty = true;
            }
        }

        #[cfg(feature = "gpu")]
        if let DeviceDrm::Gpu(manager) = &mut device.drm {
            let render_node = device.render_node.unwrap_or(self.backend_data.primary_gpu);
            let mut renderer = self
                .backend_data
                .gpus
                .single_renderer(&render_node)
                .unwrap();
            let _ = manager.try_to_restore_modifiers::<_, OutputRenderElements<
                UdevRenderer<'_>,
                WindowRenderElement<UdevRenderer<'_>>,
            >>(
                &mut renderer,
                // FIXME: For a flicker free operation we should return the actual elements for this output..
                // Instead we just use black to "simulate" a modeset :)
                &DrmOutputRenderElements::default(),
            );
        }
    }

    fn device_changed(&mut self, node: DrmNode) {
        let device = if let Some(device) = self.backend_data.backends.get_mut(&node) {
            device
        } else {
            return;
        };

        let scan_result = match device.drm_scanner.scan_connectors(device.drm.device()) {
            Ok(scan_result) => scan_result,
            Err(err) => {
                tracing::warn!(?err, "Failed to scan connectors");
                return;
            }
        };

        for event in scan_result {
            match event {
                DrmScanEvent::Connected {
                    connector,
                    crtc: Some(crtc),
                } => {
                    self.connector_connected(node, connector, crtc);
                }
                DrmScanEvent::Disconnected {
                    connector,
                    crtc: Some(crtc),
                } => {
                    self.connector_disconnected(node, connector, crtc);
                }
                _ => {}
            }
        }

        // fixup window coordinates
        self.wm_outputs_changed();
    }

    fn device_removed(&mut self, node: DrmNode) {
        let device = if let Some(device) = self.backend_data.backends.get_mut(&node) {
            device
        } else {
            return;
        };

        let crtcs: Vec<_> = device
            .drm_scanner
            .crtcs()
            .map(|(info, crtc)| (info.clone(), crtc))
            .collect();

        for (connector, crtc) in crtcs {
            self.connector_disconnected(node, connector, crtc);
        }

        debug!("Surfaces dropped");

        // drop the backends on this side
        if let Some(mut backend_data) = self.backend_data.backends.remove(&node) {
            if let Some(mut leasing_global) = backend_data.leasing_global.take() {
                leasing_global.disable_global::<EdexState<UdevData>>();
            }

            #[cfg(feature = "gpu")]
            if let Some(render_node) = backend_data.render_node {
                self.backend_data.gpus.as_mut().remove_node(&render_node);
            }

            self.handle.remove(backend_data.registration_token);

            debug!("Dropping device");
        }

        self.wm_outputs_changed();
    }

    fn frame_finish(
        &mut self,
        dev_id: DrmNode,
        crtc: crtc::Handle,
        metadata: &mut Option<DrmEventMetadata>,
    ) {
        let device_backend = match self.backend_data.backends.get_mut(&dev_id) {
            Some(backend) => backend,
            None => {
                error!("Trying to finish frame on non-existent backend {}", dev_id);
                return;
            }
        };

        let surface = match device_backend.surfaces.get_mut(&crtc) {
            Some(surface) => surface,
            None => {
                error!("Trying to finish frame on non-existent crtc {:?}", crtc);
                return;
            }
        };

        if let Some(timer_token) = surface.vblank_throttle_timer.take() {
            self.handle.remove(timer_token);
        }

        let output = if let Some(output) = self.space.outputs().find(|o| {
            o.user_data().get::<UdevOutputId>()
                == Some(&UdevOutputId {
                    device_id: surface.device_id,
                    crtc,
                })
        }) {
            output.clone()
        } else {
            // somehow we got called with an invalid output
            return;
        };

        let Some(frame_duration) = output
            .current_mode()
            .map(|mode| Duration::from_secs_f64(1_000f64 / mode.refresh as f64))
        else {
            return;
        };

        let tp = metadata.as_ref().and_then(|metadata| match metadata.time {
            smithay::backend::drm::DrmEventTime::Monotonic(tp) => tp.is_zero().not().then_some(tp),
            smithay::backend::drm::DrmEventTime::Realtime(_) => None,
        });

        let seq = metadata
            .as_ref()
            .map(|metadata| metadata.sequence)
            .unwrap_or(0);

        let (clock, flags) = if let Some(tp) = tp {
            (
                tp.into(),
                wp_presentation_feedback::Kind::Vsync
                    | wp_presentation_feedback::Kind::HwClock
                    | wp_presentation_feedback::Kind::HwCompletion,
            )
        } else {
            (self.clock.now(), wp_presentation_feedback::Kind::Vsync)
        };

        let vblank_remaining_time = surface
            .last_presentation_time
            .map(|last_presentation_time| {
                frame_duration.saturating_sub(Time::elapsed(&last_presentation_time, clock))
            });

        if let Some(vblank_remaining_time) = vblank_remaining_time {
            if vblank_remaining_time > frame_duration / 2 {
                static WARN_ONCE: Once = Once::new();
                WARN_ONCE.call_once(|| {
                    warn!("display running faster than expected, throttling vblanks and disabling HwClock")
                });
                let throttled_time = tp
                    .map(|tp| tp.saturating_add(vblank_remaining_time))
                    .unwrap_or(Duration::ZERO);
                let throttled_metadata = DrmEventMetadata {
                    sequence: seq,
                    time: DrmEventTime::Monotonic(throttled_time),
                };
                let timer_token = self
                    .handle
                    .insert_source(
                        Timer::from_duration(vblank_remaining_time),
                        move |_, _, data| {
                            data.frame_finish(dev_id, crtc, &mut Some(throttled_metadata));
                            TimeoutAction::Drop
                        },
                    )
                    .expect("failed to register vblank throttle timer");
                surface.vblank_throttle_timer = Some(timer_token);
                return;
            }
        }
        surface.last_presentation_time = Some(clock);

        let submit_result = surface.drm_output.frame_submitted();

        let schedule_render = match submit_result {
            Ok(user_data) => {
                if let Some(mut feedback) = user_data.flatten() {
                    feedback.presented(clock, Refresh::fixed(frame_duration), seq as u64, flags);
                }

                true
            }
            Err(err) => {
                warn!("Error during rendering: {:?}", err);
                match err {
                    SwapBuffersError::AlreadySwapped => true,
                    // If the device has been deactivated do not reschedule, this will be done
                    // by session resume
                    SwapBuffersError::TemporaryFailure(err)
                        if matches!(
                            err.downcast_ref::<DrmError>(),
                            Some(&DrmError::DeviceInactive)
                        ) =>
                    {
                        false
                    }
                    SwapBuffersError::TemporaryFailure(err) => matches!(
                        err.downcast_ref::<DrmError>(),
                        Some(DrmError::Access(DrmAccessError {
                            source,
                            ..
                        })) if source.kind() == io::ErrorKind::PermissionDenied
                    ),
                    SwapBuffersError::ContextLost(err) => panic!("Rendering loop lost: {}", err),
                }
            }
        };

        if schedule_render {
            let next_frame_target = clock + frame_duration;

            // What are we trying to solve by introducing a delay here:
            //
            // Basically it is all about latency of client provided buffers.
            // A client driven by frame callbacks will wait for a frame callback
            // to repaint and submit a new buffer. As we send frame callbacks
            // as part of the repaint in the compositor the latency would always
            // be approx. 2 frames. By introducing a delay before we repaint in
            // the compositor we can reduce the latency to approx. 1 frame + the
            // remaining duration from the repaint to the next VBlank.
            //
            // With the delay it is also possible to further reduce latency if
            // the client is driven by presentation feedback. As the presentation
            // feedback is directly sent after a VBlank the client can submit a
            // new buffer during the repaint delay that can hit the very next
            // VBlank, thus reducing the potential latency to below one frame.
            //
            // Choosing a good delay is a topic on its own so we just implement
            // a simple strategy here. We just split the duration between two
            // VBlanks into two steps, one for the client repaint and one for the
            // compositor repaint. Theoretically the repaint in the compositor should
            // be faster so we give the client a bit more time to repaint. On a typical
            // modern system the repaint in the compositor should not take more than 2ms
            // so this should be safe for refresh rates up to at least 120 Hz. For 120 Hz
            // this results in approx. 3.33ms time for repainting in the compositor.
            // A too big delay could result in missing the next VBlank in the compositor.
            //
            // A more complete solution could work on a sliding window analyzing past repaints
            // and do some prediction for the next repaint.
            let repaint_delay = Duration::from_secs_f64(frame_duration.as_secs_f64() * 0.6f64);

            let timer = if surface
                .render_node
                .map(|render_node| render_node != self.backend_data.primary_gpu)
                .unwrap_or(true)
            {
                // However, if we need to do a copy, that might not be enough.
                // (And without actual comparision to previous frames we cannot really know.)
                // So lets ignore that in those cases to avoid thrashing performance.
                trace!("scheduling repaint timer immediately on {:?}", crtc);
                Timer::immediate()
            } else {
                trace!(
                    "scheduling repaint timer with delay {:?} on {:?}",
                    repaint_delay,
                    crtc
                );
                Timer::from_duration(repaint_delay)
            };

            self.handle
                .insert_source(timer, move |_, _, data| {
                    data.render(dev_id, Some(crtc), next_frame_target);
                    TimeoutAction::Drop
                })
                .expect("failed to schedule frame timer");
        }
    }

    // If crtc is `Some()`, render it, else render all crtcs
    fn render(&mut self, node: DrmNode, crtc: Option<crtc::Handle>, frame_target: Time<Monotonic>) {
        let device_backend = match self.backend_data.backends.get_mut(&node) {
            Some(backend) => backend,
            None => {
                error!("Trying to render on non-existent backend {}", node);
                return;
            }
        };

        if let Some(crtc) = crtc {
            self.render_surface(node, crtc, frame_target);
        } else {
            let crtcs: Vec<_> = device_backend.surfaces.keys().copied().collect();
            for crtc in crtcs {
                self.render_surface(node, crtc, frame_target);
            }
        };
    }

    fn render_surface(&mut self, node: DrmNode, crtc: crtc::Handle, frame_target: Time<Monotonic>) {
        if !self.backend_data.dpms_on {
            return;
        }
        let output = if let Some(output) = self.space.outputs().find(|o| {
            o.user_data().get::<UdevOutputId>()
                == Some(&UdevOutputId {
                    device_id: node,
                    crtc,
                })
        }) {
            output.clone()
        } else {
            // somehow we got called with an invalid output
            return;
        };

        self.pre_repaint(&output, frame_target);

        let lock_surface = self.lock_surface_for(&output);
        let background = self.config.background;
        let screen = if self.lock.locked {
            Screen::Locked(lock_surface.as_ref())
        } else {
            Screen::Normal {
                clear: Color32F::new(background[0], background[1], background[2], 1.0),
            }
        };

        let device = if let Some(device) = self.backend_data.backends.get_mut(&node) {
            device
        } else {
            return;
        };

        let surface = if let Some(surface) = device.surfaces.get_mut(&crtc) {
            surface
        } else {
            return;
        };

        let start = Instant::now();

        // TODO get scale from the rendersurface when supporting HiDPI
        let frame = self
            .backend_data
            .pointer_image
            .get_image(1 /*scale*/, self.clock.now().into());

        let pointer_images = &mut self.backend_data.pointer_images;
        let pointer_image = pointer_images
            .iter()
            .find_map(|(image, texture)| {
                if image == &frame {
                    Some(texture.clone())
                } else {
                    None
                }
            })
            .unwrap_or_else(|| {
                let buffer = MemoryRenderBuffer::from_slice(
                    &frame.pixels_rgba,
                    Fourcc::Argb8888,
                    (frame.width as i32, frame.height as i32),
                    1,
                    Transform::Normal,
                    None,
                );
                pointer_images.push((frame, buffer.clone()));
                buffer
            });

        let mut cursor = CursorState {
            space: &self.space,
            output: &output,
            pointer_location: self.pointer.current_location(),
            pointer_image: &pointer_image,
            pointer_element: &mut self.backend_data.pointer_element,
            dnd_icon: &self.dnd_icon,
            cursor_status: &mut self.cursor_status,
        };
        let result = match &mut surface.drm_output {
            #[cfg(feature = "gpu")]
            OutputDrm::Gpu(drm_output) => {
                let primary_gpu = self.backend_data.primary_gpu;
                let render_node = surface.render_node.unwrap_or(primary_gpu);
                let mut renderer = if primary_gpu == render_node {
                    self.backend_data.gpus.single_renderer(&render_node)
                } else {
                    let format = drm_output.format();
                    self.backend_data
                        .gpus
                        .renderer(&primary_gpu, &render_node, format)
                }
                .unwrap();
                render_gpu(
                    drm_output,
                    surface.disable_direct_scanout,
                    &mut renderer,
                    &mut cursor,
                    self.show_window_preview,
                    screen,
                )
            }
            OutputDrm::Pixman(dumb) => match self.backend_data.pixman.as_mut() {
                Some(renderer) => render_pixman(
                    dumb,
                    renderer,
                    &mut cursor,
                    self.show_window_preview,
                    screen,
                ),
                None => Err(SwapBuffersError::ContextLost(
                    "no pixman renderer for a software output".into(),
                )),
            },
        };
        let reschedule = match result {
            Ok((has_rendered, states)) => {
                let dmabuf_feedback = surface.dmabuf_feedback.clone();
                self.post_repaint(&output, frame_target, dmabuf_feedback, &states);
                !has_rendered
            }
            Err(err) => {
                warn!("Error during rendering: {:#?}", err);
                match err {
                    SwapBuffersError::AlreadySwapped => false,
                    SwapBuffersError::TemporaryFailure(err) => match err.downcast_ref::<DrmError>()
                    {
                        Some(DrmError::DeviceInactive) => true,
                        Some(DrmError::Access(DrmAccessError { source, .. })) => {
                            source.kind() == io::ErrorKind::PermissionDenied
                        }
                        _ => false,
                    },
                    SwapBuffersError::ContextLost(err) => match err.downcast_ref::<DrmError>() {
                        Some(DrmError::TestFailed(_)) => {
                            // reset the complete state, disabling all connectors and planes in case we hit a test failed
                            // most likely we hit this after a tty switch when a foreign master changed CRTC <-> connector bindings
                            // and we run in a mismatch
                            device
                                .drm
                                .device_mut()
                                .reset_state()
                                .expect("failed to reset drm device");
                            true
                        }
                        _ => panic!("Rendering loop lost: {}", err),
                    },
                }
            }
        };

        if reschedule {
            let output_refresh = match output.current_mode() {
                Some(mode) => mode.refresh,
                None => return,
            };

            // If reschedule is true we either hit a temporary failure or more likely rendering
            // did not cause any damage on the output. In this case we just re-schedule a repaint
            // after approx. one frame to re-test for damage.
            let next_frame_target =
                frame_target + Duration::from_millis(1_000_000 / output_refresh as u64);
            let reschedule_timeout =
                Duration::from(next_frame_target).saturating_sub(self.clock.now().into());
            trace!(
                "reschedule repaint timer with delay {:?} on {:?}",
                reschedule_timeout,
                crtc,
            );
            let timer = Timer::from_duration(reschedule_timeout);
            self.handle
                .insert_source(timer, move |_, _, data| {
                    data.render(node, Some(crtc), next_frame_target);
                    TimeoutAction::Drop
                })
                .expect("failed to schedule frame timer");
        } else {
            let elapsed = start.elapsed();
            tracing::trace!(?elapsed, "rendered surface");
        }
    }
}

/// What the cursor and drag-and-drop icon are drawn from.
struct CursorState<'a> {
    space: &'a Space<WindowElement>,
    output: &'a Output,
    pointer_location: Point<f64, Logical>,
    pointer_image: &'a MemoryRenderBuffer,
    pointer_element: &'a mut PointerElement,
    dnd_icon: &'a Option<DndIcon>,
    cursor_status: &'a mut CursorImageStatus,
}

/// The cursor and the drag-and-drop icon, when the pointer is on this output.
fn cursor_elements<R>(
    renderer: &mut R,
    cursor: &mut CursorState<'_>,
) -> Vec<CustomRenderElements<R>>
where
    R: Renderer + ImportAll + ImportMem,
    R::TextureId: Texture + Clone + Send + 'static,
{
    let output_geometry = cursor.space.output_geometry(cursor.output).unwrap();
    let scale = Scale::from(cursor.output.current_scale().fractional_scale());

    let mut custom_elements: Vec<CustomRenderElements<R>> = Vec::new();

    if output_geometry.to_f64().contains(cursor.pointer_location) {
        let cursor_hotspot = if let CursorImageStatus::Surface(ref surface) = cursor.cursor_status {
            compositor::with_states(surface, |states| {
                states
                    .data_map
                    .get::<Mutex<CursorImageAttributes>>()
                    .unwrap()
                    .lock()
                    .unwrap()
                    .hotspot
            })
        } else {
            (0, 0).into()
        };
        let cursor_pos = cursor.pointer_location - output_geometry.loc.to_f64();

        // set cursor
        cursor
            .pointer_element
            .set_buffer(cursor.pointer_image.clone());

        // draw the cursor as relevant
        {
            // reset the cursor if the surface is no longer alive
            let mut reset = false;
            if let CursorImageStatus::Surface(ref surface) = *cursor.cursor_status {
                reset = !surface.alive();
            }
            if reset {
                *cursor.cursor_status = CursorImageStatus::default_named();
            }

            cursor
                .pointer_element
                .set_status(cursor.cursor_status.clone());
        }

        custom_elements.extend(
            cursor.pointer_element.render_elements(
                renderer,
                (cursor_pos - cursor_hotspot.to_f64())
                    .to_physical(scale)
                    .to_i32_round(),
                scale,
                1.0,
            ),
        );

        // draw the dnd icon if applicable
        if let Some(icon) = cursor.dnd_icon.as_ref() {
            let dnd_icon_pos = (cursor_pos + icon.offset.to_f64())
                .to_physical(scale)
                .to_i32_round();
            if icon.surface.alive() {
                custom_elements.extend(AsRenderElements::<R>::render_elements(
                    &SurfaceTree::from_surface(&icon.surface),
                    renderer,
                    dnd_icon_pos,
                    scale,
                    1.0,
                ));
            }
        }
    }
    custom_elements
}

/// Compose and queue a frame through Smithay's `DrmOutput` (GLES).
#[cfg(feature = "gpu")]
fn render_gpu<'a>(
    drm_output: &mut GbmDrmOutput,
    disable_direct_scanout: bool,
    renderer: &mut UdevRenderer<'a>,
    cursor: &mut CursorState<'_>,
    show_window_preview: bool,
    screen: Screen<'_>,
) -> Result<(bool, RenderElementStates), SwapBuffersError> {
    let custom_elements = cursor_elements(renderer, cursor);
    let (space, output) = (cursor.space, cursor.output);
    let (elements, clear_color) = output_elements(
        output,
        space,
        custom_elements,
        renderer,
        show_window_preview,
        screen,
    );

    let frame_mode = if disable_direct_scanout {
        FrameFlags::empty()
    } else {
        FrameFlags::DEFAULT
    };
    let (rendered, states) = drm_output
        .render_frame(renderer, &elements, clear_color, frame_mode)
        .map(|render_frame_result| (!render_frame_result.is_empty, render_frame_result.states))
        .map_err(|err| match err {
            smithay::backend::drm::compositor::RenderFrameError::PrepareFrame(err) => {
                SwapBuffersError::from(err)
            }
            smithay::backend::drm::compositor::RenderFrameError::RenderFrame(
                OutputDamageTrackerError::Rendering(err),
            ) => SwapBuffersError::from(err),
            _ => unreachable!(),
        })?;

    update_primary_scanout_output(
        space,
        output,
        cursor.dnd_icon,
        cursor.cursor_status,
        &states,
    );

    if rendered {
        let output_presentation_feedback = take_presentation_feedback(output, space, &states);
        drm_output
            .queue_frame(Some(output_presentation_feedback))
            .map_err(Into::<SwapBuffersError>::into)?;
    }

    Ok((rendered, states))
}

/// Compose a frame with pixman into a dumb buffer and flip to it.
fn render_pixman(
    dumb: &mut DumbOutput<Option<OutputPresentationFeedback>>,
    renderer: &mut PixmanRenderer,
    cursor: &mut CursorState<'_>,
    show_window_preview: bool,
    screen: Screen<'_>,
) -> Result<(bool, RenderElementStates), SwapBuffersError> {
    let custom_elements = cursor_elements(renderer, cursor);
    let (space, output) = (cursor.space, cursor.output);
    let (elements, clear_color) = output_elements(
        output,
        space,
        custom_elements,
        renderer,
        show_window_preview,
        screen,
    );
    let frame = dumb
        .render_frame(renderer, &elements, clear_color)
        .map_err(dumb_swap_error)?;

    update_primary_scanout_output(
        space,
        output,
        cursor.dnd_icon,
        cursor.cursor_status,
        &frame.states,
    );

    if frame.rendered {
        let output_presentation_feedback = take_presentation_feedback(output, space, &frame.states);
        dumb.queue_frame(Some(output_presentation_feedback))
            .map_err(dumb_swap_error)?;
    }

    Ok((frame.rendered, frame.states))
}

fn dumb_swap_error(err: DumbError) -> SwapBuffersError {
    match err {
        DumbError::Drm(err) => err.into(),
        err => SwapBuffersError::TemporaryFailure(Box::new(err)),
    }
}

/// Smithay's `DrmOutput` for a CRTC: GBM swapchain, GLES composition, plane scan-out.
#[cfg(feature = "gpu")]
fn init_gpu_output(
    manager: &mut GbmDrmOutputManager,
    crtc: crtc::Handle,
    drm_mode: smithay::reexports::drm::control::Mode,
    connector: &connector::Info,
    output: &Output,
    renderer: &mut UdevRenderer<'_>,
) -> Result<GbmDrmOutput, String> {
    let drm_device = manager.device();
    let driver = drm_device
        .get_driver()
        .map_err(|err| format!("Failed to query drm driver: {err}"))?;
    let mut planes = drm_device
        .planes(&crtc)
        .map_err(|err| format!("Failed to query crtc planes: {err}"))?;

    // Using an overlay plane on a nvidia card breaks
    if driver
        .name()
        .to_string_lossy()
        .to_lowercase()
        .contains("nvidia")
        || driver
            .description()
            .to_string_lossy()
            .to_lowercase()
            .contains("nvidia")
    {
        planes.overlay = vec![];
    }

    manager
        .initialize_output::<_, OutputRenderElements<UdevRenderer<'_>, WindowRenderElement<UdevRenderer<'_>>>>(
            crtc,
            drm_mode,
            &[connector.handle()],
            output,
            Some(planes),
            renderer,
            &DrmOutputRenderElements::default(),
        )
        .map_err(|err| err.to_string())
}

/// The connector mode a rule asks for: exact size (and refresh when given), else preferred.
fn pick_mode(
    connector: &connector::Info,
    rule: crate::config::ModeRule,
) -> Option<smithay::reexports::drm::control::Mode> {
    let modes = connector.modes();
    let preferred = modes
        .iter()
        .find(|m| m.mode_type().contains(ModeTypeFlags::PREFERRED))
        .or(modes.first())
        .copied();
    match rule {
        crate::config::ModeRule::Preferred => preferred,
        crate::config::ModeRule::Exact { w, h, refresh_hz } => modes
            .iter()
            .filter(|m| m.size() == (w as u16, h as u16))
            .min_by_key(|m| {
                let hz = m.vrefresh() as f32;
                ((refresh_hz.map(|r| (r - hz).abs()).unwrap_or(-hz)) * 1000.0) as i64
            })
            .copied()
            .or(preferred),
    }
}

/// Load a night-light gamma ramp into a CRTC.
fn set_crtc_gamma(device: Option<&DrmDevice>, crtc: crtc::Handle, kelvin: Option<u32>) {
    let Some(device) = device else {
        return;
    };
    let Ok(info) = device.get_crtc(crtc) else {
        return;
    };
    let size = info.gamma_length() as usize;
    if size == 0 {
        return;
    }
    let (r, g, b) = crate::gamma::ramps(size, kelvin);
    if let Err(e) = device.set_gamma(crtc, &r, &g, &b) {
        warn!("setting gamma on {crtc:?}: {e}");
    }
}

impl EdexState<UdevData> {
    fn udev_set_gamma(&mut self, kelvin: Option<u32>) {
        self.backend_data.gamma = kelvin;
        for backend in self.backend_data.backends.values() {
            for crtc in backend.surfaces.keys() {
                set_crtc_gamma(Some(backend.drm.device()), *crtc, kelvin);
            }
        }
    }

    fn udev_set_dpms(&mut self, on: bool) {
        if self.backend_data.dpms_on == on {
            return;
        }
        self.backend_data.dpms_on = on;
        let mut nodes = Vec::new();
        for (node, backend) in self.backend_data.backends.iter_mut() {
            for surface in backend.surfaces.values_mut() {
                if !on {
                    if let Err(e) = surface.drm_output.clear() {
                        warn!("turning the display off: {e}");
                    }
                } else {
                    surface.drm_output.reset_buffers();
                }
            }
            nodes.push(*node);
        }
        if on {
            for node in nodes {
                self.handle
                    .insert_idle(move |state| state.render(node, None, state.clock.now()));
            }
        }
    }

    /// Re-apply output rules: scale, transform and position change live; a new mode is set
    /// through a modeset; enabling or disabling an output re-runs connector setup.
    fn udev_apply_output_config(&mut self) {
        // Outputs switched off by the settings.
        let mut to_disable = Vec::new();
        for (node, backend) in self.backend_data.backends.iter() {
            for (crtc, surface) in backend.surfaces.iter() {
                let name = format!(
                    "{}-{}",
                    surface.connector.interface().as_str(),
                    surface.connector.interface_id()
                );
                if self.config.output_rule(&name).disabled {
                    to_disable.push((*node, surface.connector.clone(), *crtc));
                }
            }
        }
        for (node, connector, crtc) in to_disable {
            self.connector_disconnected(node, connector.clone(), crtc);
            self.backend_data
                .disabled_connectors
                .push((node, connector, crtc));
        }
        // Outputs switched back on.
        let pending = std::mem::take(&mut self.backend_data.disabled_connectors);
        for (node, connector, crtc) in pending {
            self.connector_connected(node, connector, crtc);
        }

        let outputs: Vec<Output> = self.space.outputs().cloned().collect();
        let mut x = 0;
        for output in outputs {
            let rule = self.config.output_rule(&output.name());
            let Some(id) = output.user_data().get::<UdevOutputId>() else {
                continue;
            };
            let (node, crtc) = (id.device_id, id.crtc);
            // Mode
            let wanted = self
                .backend_data
                .backends
                .get(&node)
                .and_then(|b| b.surfaces.get(&crtc))
                .and_then(|s| pick_mode(&s.connector, rule.mode));
            if let Some(mode) = wanted {
                let wl_mode = WlMode::from(mode);
                if output.current_mode() != Some(wl_mode) {
                    match self.udev_use_mode(node, crtc, mode) {
                        Ok(()) => output.change_current_state(Some(wl_mode), None, None, None),
                        Err(e) => warn!("mode change on {} failed: {e}", output.name()),
                    }
                }
            }
            let position = rule.position.unwrap_or((x, 0));
            output.change_current_state(
                None,
                Some(crate::manage::transform_from_index(rule.transform)),
                Some(smithay::output::Scale::Fractional(rule.scale)),
                Some(position.into()),
            );
            self.space.map_output(&output, position);
            if let Some(geo) = self.space.output_geometry(&output) {
                x = x.max(geo.loc.x + geo.size.w);
            }
            self.backend_data.reset_buffers(&output);
        }
        self.wm_outputs_changed();
    }

    /// Modeset a CRTC to `mode` (on its next frame).
    fn udev_use_mode(
        &mut self,
        node: DrmNode,
        crtc: crtc::Handle,
        mode: smithay::reexports::drm::control::Mode,
    ) -> Result<(), String> {
        let Some(backend) = self.backend_data.backends.get_mut(&node) else {
            return Err("no such device".into());
        };
        let render_node = backend.render_node;
        let Some(surface) = backend.surfaces.get_mut(&crtc) else {
            return Err("no such output".into());
        };
        match &mut surface.drm_output {
            #[cfg(feature = "gpu")]
            OutputDrm::Gpu(drm_output) => {
                let render_node = render_node.unwrap_or(self.backend_data.primary_gpu);
                let mut renderer = self
                    .backend_data
                    .gpus
                    .single_renderer(&render_node)
                    .map_err(|e| e.to_string())?;
                drm_output
                    .use_mode::<_, OutputRenderElements<
                        UdevRenderer<'_>,
                        WindowRenderElement<UdevRenderer<'_>>,
                    >>(mode, &mut renderer, &DrmOutputRenderElements::default())
                    .map_err(|e| e.to_string())
            }
            OutputDrm::Pixman(dumb) => {
                let _ = render_node;
                dumb.use_mode(mode).map_err(|e| e.to_string())
            }
        }
    }

    fn udev_screenshot(
        &mut self,
        output: Option<&str>,
        region: Option<Rectangle<i32, Logical>>,
        path: &str,
    ) -> anyhow::Result<()> {
        let target = match (output, region) {
            (Some(name), _) => self.space.outputs().find(|o| o.name() == name).cloned(),
            (None, Some(r)) => self
                .space
                .outputs()
                .find(|o| self.space.output_geometry(o).is_some_and(|g| g.overlaps(r)))
                .cloned(),
            (None, None) => self
                .wm
                .focused_output_name()
                .and_then(|n| self.space.outputs().find(|o| o.name() == n))
                .cloned(),
        }
        .ok_or_else(|| anyhow::anyhow!("no such output"))?;
        let background = self.config.background;
        let lock_surface = self.lock_surface_for(&target);
        let screen = if self.lock.locked {
            Screen::Locked(lock_surface.as_ref())
        } else {
            Screen::Normal {
                clear: Color32F::new(background[0], background[1], background[2], 1.0),
            }
        };
        let img = match self.backend_data.pixman.as_mut() {
            Some(renderer) => crate::screenshot::capture::<
                _,
                smithay::reexports::pixman::Image<'static, 'static>,
            >(renderer, &target, &self.space, screen)?,
            #[cfg(feature = "gpu")]
            None => {
                let primary = self.backend_data.primary_gpu;
                let mut renderer = self
                    .backend_data
                    .gpus
                    .single_renderer(&primary)
                    .map_err(|e| anyhow::anyhow!("{e}"))?;
                crate::screenshot::capture::<_, smithay::backend::renderer::gles::GlesTexture>(
                    &mut renderer,
                    &target,
                    &self.space,
                    screen,
                )?
            }
            #[cfg(not(feature = "gpu"))]
            None => anyhow::bail!("no renderer"),
        };
        let img = match (region, self.space.output_geometry(&target)) {
            (Some(r), Some(geo)) => {
                crate::screenshot::crop(img, geo, target.current_scale().fractional_scale(), r)?
            }
            _ => img,
        };
        crate::screenshot::save(&img, path)
    }
}
