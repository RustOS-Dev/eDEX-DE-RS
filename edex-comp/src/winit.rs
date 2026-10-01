use std::{
    sync::{atomic::Ordering, Mutex},
    time::Duration,
};

use smithay::backend::renderer::ImportEgl;

use smithay::{
    backend::{
        allocator::dmabuf::Dmabuf,
        egl::EGLDevice,
        renderer::{
            damage::{Error as OutputDamageTrackerError, OutputDamageTracker},
            element::AsRenderElements,
            gles::GlesRenderer,
            ImportDma, ImportMemWl,
        },
        winit::{self, WinitEvent, WinitGraphicsBackend},
        SwapBuffersError,
    },
    delegate_dmabuf,
    input::{
        keyboard::LedState,
        pointer::{CursorImageAttributes, CursorImageStatus},
    },
    output::{Mode, Output, PhysicalProperties, Subpixel},
    reexports::{
        calloop::EventLoop,
        wayland_protocols::wp::presentation_time::server::wp_presentation_feedback,
        wayland_server::{protocol::wl_surface, Display},
        winit::platform::pump_events::PumpStatus,
    },
    utils::{IsAlive, Scale, Transform},
    wayland::{
        compositor,
        dmabuf::{
            DmabufFeedback, DmabufFeedbackBuilder, DmabufGlobal, DmabufHandler, DmabufState,
            ImportNotifier,
        },
        presentation::Refresh,
    },
};
use tracing::{error, info, warn};

use crate::state::{take_presentation_feedback, Backend, EdexState, InitOptions};
use crate::{drawing::*, render::*};
use smithay::backend::renderer::Color32F;

pub const OUTPUT_NAME: &str = "winit";

pub struct WinitData {
    backend: WinitGraphicsBackend<GlesRenderer>,
    damage_tracker: OutputDamageTracker,
    dmabuf_state: (DmabufState, DmabufGlobal, Option<DmabufFeedback>),
    full_redraw: u8,
}

impl DmabufHandler for EdexState<WinitData> {
    fn dmabuf_state(&mut self) -> &mut DmabufState {
        &mut self.backend_data.dmabuf_state.0
    }

    fn dmabuf_imported(
        &mut self,
        _global: &DmabufGlobal,
        dmabuf: Dmabuf,
        notifier: ImportNotifier,
    ) {
        if self
            .backend_data
            .backend
            .renderer()
            .import_dmabuf(&dmabuf, None)
            .is_ok()
        {
            let _ = notifier.successful::<EdexState<WinitData>>();
        } else {
            notifier.failed();
        }
    }
}
delegate_dmabuf!(EdexState<WinitData>);

impl Backend for WinitData {
    fn seat_name(&self) -> String {
        String::from("winit")
    }
    fn reset_buffers(&mut self, _output: &Output) {
        self.full_redraw = 4;
    }
    fn early_import(&mut self, _surface: &wl_surface::WlSurface) {}
    fn update_led_state(&mut self, _led_state: LedState) {}

    fn reset_buffers_for(state: &mut EdexState<Self>, _output: &Output) {
        state.backend_data.full_redraw = 4;
    }

    fn screenshot(
        state: &mut EdexState<Self>,
        _output: Option<&str>,
        region: Option<smithay::utils::Rectangle<i32, smithay::utils::Logical>>,
        path: &str,
    ) -> anyhow::Result<()> {
        let output = state
            .space
            .outputs()
            .next()
            .cloned()
            .ok_or_else(|| anyhow::anyhow!("no output"))?;
        let c = state.config.background;
        let clear = Color32F::new(c[0], c[1], c[2], 1.0);
        let renderer = state.backend_data.backend.renderer();
        let img = crate::screenshot::capture::<_, smithay::backend::renderer::gles::GlesTexture>(
            renderer,
            &output,
            &state.space,
            clear,
        )?;
        let img = match (region, state.space.output_geometry(&output)) {
            (Some(r), Some(geo)) => {
                crate::screenshot::crop(img, geo, output.current_scale().fractional_scale(), r)?
            }
            _ => img,
        };
        crate::screenshot::save(&img, path)
    }
}

pub fn run_winit(opts: InitOptions) {
    let mut event_loop = EventLoop::try_new().unwrap();
    let display = Display::new().unwrap();
    let mut display_handle = display.handle();

    let (mut backend, mut winit) = match winit::init::<GlesRenderer>() {
        Ok(ret) => ret,
        Err(err) => {
            error!("Failed to initialize Winit backend: {}", err);
            return;
        }
    };
    let size = backend.window_size();

    let mode = Mode {
        size,
        refresh: 60_000,
    };
    let output = Output::new(
        OUTPUT_NAME.to_string(),
        PhysicalProperties {
            size: (0, 0).into(),
            subpixel: Subpixel::Unknown,
            make: "Smithay".into(),
            model: "Winit".into(),
        },
    );
    let _global = output.create_global::<EdexState<WinitData>>(&display.handle());
    output.change_current_state(
        Some(mode),
        Some(Transform::Flipped180),
        None,
        Some((0, 0).into()),
    );
    output.set_preferred(mode);

    let render_node = EGLDevice::device_for_display(backend.renderer().egl_context().display())
        .and_then(|device| device.try_get_render_node());

    let dmabuf_default_feedback = match render_node {
        Ok(Some(node)) => {
            let dmabuf_formats = backend.renderer().dmabuf_formats();
            let dmabuf_default_feedback = DmabufFeedbackBuilder::new(node.dev_id(), dmabuf_formats)
                .build()
                .unwrap();
            Some(dmabuf_default_feedback)
        }
        Ok(None) => {
            warn!("failed to query render node, dmabuf will use v3");
            None
        }
        Err(err) => {
            warn!(?err, "failed to egl device for display, dmabuf will use v3");
            None
        }
    };

    // if we failed to build dmabuf feedback we fall back to dmabuf v3
    // Note: egl on Mesa requires either v4 or wl_drm (initialized with bind_wl_display)
    let dmabuf_state = if let Some(default_feedback) = dmabuf_default_feedback {
        let mut dmabuf_state = DmabufState::new();
        let dmabuf_global = dmabuf_state
            .create_global_with_default_feedback::<EdexState<WinitData>>(
                &display.handle(),
                &default_feedback,
            );
        (dmabuf_state, dmabuf_global, Some(default_feedback))
    } else {
        let dmabuf_formats = backend.renderer().dmabuf_formats();
        let mut dmabuf_state = DmabufState::new();
        let dmabuf_global =
            dmabuf_state.create_global::<EdexState<WinitData>>(&display.handle(), dmabuf_formats);
        (dmabuf_state, dmabuf_global, None)
    };

    if backend
        .renderer()
        .bind_wl_display(&display.handle())
        .is_ok()
    {
        info!("EGL hardware-acceleration enabled");
    };

    let data = {
        let damage_tracker = OutputDamageTracker::from_output(&output);

        WinitData {
            backend,
            damage_tracker,
            dmabuf_state,
            full_redraw: 0,
        }
    };
    let greeter = opts.greeter;
    let mut state = EdexState::init(display, event_loop.handle(), data, true, opts);
    state
        .shm_state
        .update_formats(state.backend_data.backend.renderer().shm_formats());
    state.space.map_output(&output, (0, 0));
    state.wm_output_added(&output);
    state.apply_config();

    if greeter {
        state.start_greeter();
    } else {
        state.start_direct_session();
    }

    info!("Initialization completed, starting the main loop.");

    let mut pointer_element = PointerElement::default();

    while state.running.load(Ordering::SeqCst) {
        let status = winit.dispatch_new_events(|event| match event {
            WinitEvent::Resized { size, .. } => {
                // We only have one output
                let output = state.space.outputs().next().unwrap().clone();
                state.space.map_output(&output, (0, 0));
                let mode = Mode {
                    size,
                    refresh: 60_000,
                };
                output.change_current_state(Some(mode), None, None, None);
                output.set_preferred(mode);
                state.wm_outputs_changed();
            }
            WinitEvent::Input(event) => state.process_input_event_windowed(event, OUTPUT_NAME),
            _ => (),
        });

        if let PumpStatus::Exit(_) = status {
            state.running.store(false, Ordering::SeqCst);
            break;
        }

        // drawing logic
        {
            let now = state.clock.now();
            let frame_target = now
                + output
                    .current_mode()
                    .map(|mode| Duration::from_secs_f64(1_000f64 / mode.refresh as f64))
                    .unwrap_or_default();
            state.pre_repaint(&output, frame_target);

            let lock_surface = state.lock_surface_for(&output);
            let locked = state.lock.locked;
            let background = state.config.background;
            let screen = if locked {
                Screen::Locked(lock_surface.as_ref())
            } else {
                Screen::Normal {
                    clear: Color32F::new(background[0], background[1], background[2], 1.0),
                }
            };

            let backend = &mut state.backend_data.backend;

            // draw the cursor as relevant
            // reset the cursor if the surface is no longer alive
            let mut reset = false;
            if let CursorImageStatus::Surface(ref surface) = state.cursor_status {
                reset = !surface.alive();
            }
            if reset {
                state.cursor_status = CursorImageStatus::default_named();
            }
            let cursor_visible = !matches!(state.cursor_status, CursorImageStatus::Surface(_));

            pointer_element.set_status(state.cursor_status.clone());

            let full_redraw = &mut state.backend_data.full_redraw;
            *full_redraw = full_redraw.saturating_sub(1);
            let space = &mut state.space;
            let damage_tracker = &mut state.backend_data.damage_tracker;
            let show_window_preview = state.show_window_preview;

            let dnd_icon = state.dnd_icon.as_ref();

            let scale = Scale::from(output.current_scale().fractional_scale());
            let cursor_hotspot =
                if let CursorImageStatus::Surface(ref surface) = state.cursor_status {
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
            let cursor_pos = state.pointer.current_location();

            let age = if *full_redraw > 0 {
                0
            } else {
                backend.buffer_age().unwrap_or(0)
            };
            let render_res = backend.bind().and_then(|(renderer, mut fb)| {
                let mut elements = Vec::<CustomRenderElements<GlesRenderer>>::new();

                elements.extend(
                    pointer_element.render_elements(
                        renderer,
                        (cursor_pos - cursor_hotspot.to_f64())
                            .to_physical(scale)
                            .to_i32_round(),
                        scale,
                        1.0,
                    ),
                );

                // draw the dnd icon if any
                if let Some(icon) = dnd_icon {
                    let dnd_icon_pos = (cursor_pos + icon.offset.to_f64())
                        .to_physical(scale)
                        .to_i32_round();
                    if icon.surface.alive() {
                        elements.extend(AsRenderElements::<GlesRenderer>::render_elements(
                            &smithay::desktop::space::SurfaceTree::from_surface(&icon.surface),
                            renderer,
                            dnd_icon_pos,
                            scale,
                            1.0,
                        ));
                    }
                }

                let res = render_output(
                    &output,
                    space,
                    elements,
                    renderer,
                    &mut fb,
                    damage_tracker,
                    age,
                    show_window_preview,
                    screen,
                )
                .map_err(|err| match err {
                    OutputDamageTrackerError::Rendering(err) => err.into(),
                    _ => unreachable!(),
                });

                res
            });

            match render_res {
                Ok(render_output_result) => {
                    let has_rendered = render_output_result.damage.is_some();
                    if let Some(damage) = render_output_result.damage {
                        if let Err(err) = backend.submit(Some(damage)) {
                            warn!("Failed to submit buffer: {}", err);
                        }
                    }

                    backend.window().set_cursor_visible(cursor_visible);

                    let states = render_output_result.states;
                    if has_rendered {
                        let mut output_presentation_feedback =
                            take_presentation_feedback(&output, &state.space, &states);
                        output_presentation_feedback.presented(
                            frame_target,
                            output
                                .current_mode()
                                .map(|mode| {
                                    Refresh::fixed(Duration::from_secs_f64(
                                        1_000f64 / mode.refresh as f64,
                                    ))
                                })
                                .unwrap_or(Refresh::Unknown),
                            0,
                            wp_presentation_feedback::Kind::Vsync,
                        )
                    }

                    // Send frame events so that client start drawing their next frame
                    state.post_repaint(&output, frame_target, None, &states);
                }
                Err(SwapBuffersError::ContextLost(err)) => {
                    error!("Critical Rendering Error: {}", err);
                    state.running.store(false, Ordering::SeqCst);
                }
                Err(err) => warn!("Rendering error: {}", err),
            }
        }

        let result = event_loop.dispatch(Some(Duration::from_millis(1)), &mut state);
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
