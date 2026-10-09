//! Application windows through `zwlr_foreign_toplevel_management_v1` (labwc, sway, wayfire…):
//! the list the centre tab strip shows on compositors other than Hyprland, and the requests
//! behind its controls (activate, minimize, maximize, fullscreen, close).
//!
//! The protocol objects live on the platform's event queue; [`Toplevels`] is a cheap shared
//! handle the window-manager backend keeps to read the list and send requests from anywhere.

use std::{
    collections::BTreeMap,
    sync::{
        atomic::{AtomicU64, Ordering},
        Arc, Mutex,
    },
};

use smithay_client_toolkit::dispatch2::Dispatch2;
use wayland_client::{
    backend::ObjectData,
    protocol::{wl_output, wl_seat},
    Connection, Proxy, QueueHandle,
};
use wayland_protocols_wlr::foreign_toplevel::v1::client::{
    zwlr_foreign_toplevel_handle_v1::{self, ZwlrForeignToplevelHandleV1},
    zwlr_foreign_toplevel_manager_v1::{self, ZwlrForeignToplevelManagerV1},
};

use crate::{Platform, PlatformEvent};

/// A window as the compositor last described it (after its `done` event).
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Toplevel {
    /// Stable for the window's lifetime; never reused.
    pub id: u64,
    pub app_id: String,
    pub title: String,
    pub maximized: bool,
    pub minimized: bool,
    pub activated: bool,
    pub fullscreen: bool,
    /// Names of the outputs the window is on (`wl_output.name`, e.g. `Virtual-1`).
    pub outputs: Vec<String>,
    /// Transient for another window (dialogs): such windows get no tab of their own.
    pub parent: Option<u64>,
}

struct Entry {
    handle: ZwlrForeignToplevelHandleV1,
    /// State as of the last `done`.
    current: Toplevel,
    /// Changes since then.
    pending: Toplevel,
    /// Seen a first `done`: until then the window is not listed.
    mapped: bool,
}

#[derive(Default)]
struct Inner {
    entries: BTreeMap<u64, Entry>,
    seat: Option<wl_seat::WlSeat>,
    output_names: Vec<(wl_output::WlOutput, String)>,
    /// The manager sent `finished` (or the compositor never offered it).
    finished: bool,
}

/// Shared view of the compositor's windows. Requests are queued on the Wayland connection and
/// flushed with the next dispatch.
#[derive(Clone, Default)]
pub struct Toplevels {
    inner: Arc<Mutex<Inner>>,
}

static NEXT_ID: AtomicU64 = AtomicU64::new(1);

impl Toplevels {
    /// Windows in the order the compositor announced them (oldest first).
    pub fn list(&self) -> Vec<Toplevel> {
        let inner = self.inner.lock().unwrap();
        inner
            .entries
            .values()
            .filter(|e| e.mapped)
            .map(|e| e.current.clone())
            .collect()
    }

    /// Whether the compositor still sends window updates.
    pub fn active(&self) -> bool {
        !self.inner.lock().unwrap().finished
    }

    fn with_handle(&self, id: u64, f: impl FnOnce(&ZwlrForeignToplevelHandleV1, &Inner)) -> bool {
        let inner = self.inner.lock().unwrap();
        match inner.entries.get(&id) {
            Some(e) => {
                f(&e.handle, &inner);
                true
            }
            None => false,
        }
    }

    /// Raise and focus the window (also un-minimizes it on most compositors).
    pub fn activate(&self, id: u64) -> bool {
        self.with_handle(id, |h, inner| {
            if let Some(seat) = &inner.seat {
                h.activate(seat);
            }
        })
    }

    pub fn set_minimized(&self, id: u64, on: bool) -> bool {
        self.with_handle(id, |h, _| {
            if on {
                h.set_minimized()
            } else {
                h.unset_minimized()
            }
        })
    }

    pub fn set_maximized(&self, id: u64, on: bool) -> bool {
        self.with_handle(id, |h, _| {
            if on {
                h.set_maximized()
            } else {
                h.unset_maximized()
            }
        })
    }

    pub fn set_fullscreen(&self, id: u64, on: bool) -> bool {
        self.with_handle(id, |h, _| {
            if h.version() < 2 {
                return;
            }
            if on {
                h.set_fullscreen(None)
            } else {
                h.unset_fullscreen()
            }
        })
    }

    pub fn close(&self, id: u64) -> bool {
        self.with_handle(id, |h, _| h.close())
    }

    /// Use `seat` for activation unless one is already set.
    pub(crate) fn set_seat(&self, seat: Option<wl_seat::WlSeat>) {
        let mut inner = self.inner.lock().unwrap();
        if inner.seat.is_none() {
            inner.seat = seat;
        }
    }

    pub(crate) fn replace_seat(&self, seat: Option<wl_seat::WlSeat>) {
        self.inner.lock().unwrap().seat = seat;
    }

    pub(crate) fn set_output_name(&self, output: &wl_output::WlOutput, name: Option<String>) {
        let mut inner = self.inner.lock().unwrap();
        inner.output_names.retain(|(o, _)| o != output);
        if let Some(n) = name {
            inner.output_names.push((output.clone(), n));
        }
    }

    pub(crate) fn mark_finished(&self) {
        self.inner.lock().unwrap().finished = true;
    }
}

/// User data of the manager object.
#[derive(Debug, Clone, Copy, Default)]
pub struct ToplevelManagerData;

/// User data of a toplevel handle: its id.
#[derive(Debug, Clone, Copy)]
pub struct ToplevelHandleData(u64);

impl<E: 'static> Dispatch2<ZwlrForeignToplevelManagerV1, Platform<E>> for ToplevelManagerData {
    fn event(
        &self,
        state: &mut Platform<E>,
        _: &ZwlrForeignToplevelManagerV1,
        event: zwlr_foreign_toplevel_manager_v1::Event,
        _: &Connection,
        _: &QueueHandle<Platform<E>>,
    ) {
        match event {
            zwlr_foreign_toplevel_manager_v1::Event::Toplevel { toplevel } => {
                let id = toplevel
                    .data::<ToplevelHandleData>()
                    .map(|d| d.0)
                    .unwrap_or(0);
                let mut inner = state.toplevels.inner.lock().unwrap();
                inner.entries.insert(
                    id,
                    Entry {
                        handle: toplevel,
                        current: Toplevel {
                            id,
                            ..Default::default()
                        },
                        pending: Toplevel {
                            id,
                            ..Default::default()
                        },
                        mapped: false,
                    },
                );
            }
            zwlr_foreign_toplevel_manager_v1::Event::Finished => {
                state.toplevels.mark_finished();
                state.events.push_back(PlatformEvent::ToplevelsChanged);
            }
            _ => {}
        }
    }

    fn event_created_child(opcode: u16, qh: &QueueHandle<Platform<E>>) -> Arc<dyn ObjectData> {
        match opcode {
            zwlr_foreign_toplevel_manager_v1::EVT_TOPLEVEL_OPCODE => qh
                .make_data::<ZwlrForeignToplevelHandleV1, _>(ToplevelHandleData(
                    NEXT_ID.fetch_add(1, Ordering::Relaxed),
                )),
            _ => panic!(
                "unexpected child object for zwlr_foreign_toplevel_manager_v1 event {opcode}"
            ),
        }
    }
}

impl<E: 'static> Dispatch2<ZwlrForeignToplevelHandleV1, Platform<E>> for ToplevelHandleData {
    fn event(
        &self,
        state: &mut Platform<E>,
        handle: &ZwlrForeignToplevelHandleV1,
        event: zwlr_foreign_toplevel_handle_v1::Event,
        _: &Connection,
        _: &QueueHandle<Platform<E>>,
    ) {
        use zwlr_foreign_toplevel_handle_v1::Event as Ev;
        let mut changed = false;
        {
            let mut inner = state.toplevels.inner.lock().unwrap();
            let output_name = |inner: &Inner, o: &wl_output::WlOutput| {
                inner
                    .output_names
                    .iter()
                    .find(|(w, _)| w == o)
                    .map(|(_, n)| n.clone())
                    .unwrap_or_else(|| format!("output-{}", o.id().protocol_id()))
            };
            match event {
                Ev::Title { title } => {
                    if let Some(e) = inner.entries.get_mut(&self.0) {
                        e.pending.title = title;
                    }
                }
                Ev::AppId { app_id } => {
                    if let Some(e) = inner.entries.get_mut(&self.0) {
                        e.pending.app_id = app_id;
                    }
                }
                Ev::OutputEnter { output } => {
                    let name = output_name(&inner, &output);
                    if let Some(e) = inner.entries.get_mut(&self.0) {
                        if !e.pending.outputs.contains(&name) {
                            e.pending.outputs.push(name);
                        }
                    }
                }
                Ev::OutputLeave { output } => {
                    let name = output_name(&inner, &output);
                    if let Some(e) = inner.entries.get_mut(&self.0) {
                        e.pending.outputs.retain(|n| n != &name);
                    }
                }
                Ev::State { state: raw } => {
                    if let Some(e) = inner.entries.get_mut(&self.0) {
                        let states: Vec<u32> = raw
                            .chunks_exact(4)
                            .map(|c| u32::from_ne_bytes([c[0], c[1], c[2], c[3]]))
                            .collect();
                        use zwlr_foreign_toplevel_handle_v1::State as S;
                        let has = |s: S| states.contains(&(s as u32));
                        e.pending.maximized = has(S::Maximized);
                        e.pending.minimized = has(S::Minimized);
                        e.pending.activated = has(S::Activated);
                        e.pending.fullscreen = has(S::Fullscreen);
                    }
                }
                Ev::Parent { parent } => {
                    let pid = parent
                        .as_ref()
                        .and_then(|p| p.data::<ToplevelHandleData>())
                        .map(|d| d.0);
                    if let Some(e) = inner.entries.get_mut(&self.0) {
                        e.pending.parent = pid;
                    }
                }
                Ev::Done => {
                    if let Some(e) = inner.entries.get_mut(&self.0) {
                        if !e.mapped || e.current != e.pending {
                            e.current = e.pending.clone();
                            e.mapped = true;
                            changed = true;
                        }
                    }
                }
                Ev::Closed => {
                    if let Some(e) = inner.entries.remove(&self.0) {
                        changed = e.mapped;
                    }
                    handle.destroy();
                }
                _ => {}
            }
        }
        if changed {
            state.events.push_back(PlatformEvent::ToplevelsChanged);
        }
    }
}
