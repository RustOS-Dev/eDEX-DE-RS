//! Window management model: workspaces, tiling inside the shell's app area, minimize, maximize,
//! fullscreen, floating and focus.
//!
//! The model is pure: it is generic over the window handle `W` and produces [`Placement`]s that
//! the compositor applies to its `Space`. Keeping Smithay out of it lets the behaviour the shell
//! depends on (tabs, the centre slot, maximize stepping the side panels aside) be unit-tested.

use comp_proto::{
    Direction, Monitor, Rect, Window, WindowId, WindowMode, Workspace, WorkspaceId,
    WorkspaceTarget, MINIMIZED_WORKSPACE, SCRATCH_WORKSPACE,
};

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum TileLayout {
    /// Each new window splits the last one along its longer side.
    #[default]
    Dwindle,
    /// The first window takes the left half; the others stack on the right.
    Master,
}

impl TileLayout {
    pub fn from_name(name: &str) -> Self {
        match name {
            "master" => TileLayout::Master,
            _ => TileLayout::Dwindle,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct WmConfig {
    pub gaps_in: i32,
    pub gaps_out: i32,
    pub layout: TileLayout,
    /// Number of regular workspaces (1..=workspaces).
    pub workspaces: i32,
}

impl Default for WmConfig {
    fn default() -> Self {
        Self {
            gaps_in: 4,
            gaps_out: 8,
            layout: TileLayout::Dwindle,
            workspaces: 10,
        }
    }
}

/// Static facts about a window supplied by the compositor.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct WindowInfo {
    pub app_id: String,
    pub title: String,
    pub xwayland: bool,
    pub pid: Option<i32>,
    /// Dialogs and fixed-size windows start floating.
    pub wants_float: bool,
    /// Size the client asked for, used when it floats.
    pub preferred: Option<(i32, i32)>,
}

#[derive(Clone, Debug)]
struct Managed<W> {
    id: WindowId,
    handle: W,
    info: WindowInfo,
    workspace: WorkspaceId,
    mode: WindowMode,
    /// Mode to return to when leaving fullscreen or maximized.
    restore_mode: WindowMode,
    minimized: bool,
    urgent: bool,
    float_rect: Option<Rect>,
}

#[derive(Clone, Debug, PartialEq)]
struct OutputSlot {
    name: String,
    description: String,
    /// Global logical geometry.
    geometry: Rect,
    /// Shell-reported areas, relative to the output; empty = whole output.
    tiled: Rect,
    maximized: Rect,
    active: WorkspaceId,
    scale: f32,
    transform: u32,
    refresh_rate: f32,
    mode_px: (u32, u32),
    modes: Vec<String>,
    dpms: bool,
}

/// Where and how one window is shown.
#[derive(Clone, Debug, PartialEq)]
pub struct Placement<W> {
    pub id: WindowId,
    pub handle: W,
    /// Global logical rectangle of the window geometry.
    pub rect: Rect,
    pub visible: bool,
    pub mode: WindowMode,
    pub activated: bool,
    /// Which edges touch a neighbour or the area border (for xdg tiled states).
    pub tiled: bool,
    /// Draw above everything else on its output.
    pub on_top: bool,
}

/// Output facts the compositor reports when an output appears or changes.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct OutputInfo {
    pub name: String,
    pub description: String,
    pub geometry: Rect,
    pub scale: f32,
    pub transform: u32,
    pub refresh_rate: f32,
    pub mode_px: (u32, u32),
    pub modes: Vec<String>,
}

#[derive(Debug)]
pub struct Wm<W> {
    windows: Vec<Managed<W>>,
    outputs: Vec<OutputSlot>,
    focused_output: usize,
    focus: Option<WindowId>,
    history: Vec<WindowId>,
    scratch_visible: bool,
    /// Last output each workspace was shown on.
    homes: Vec<(WorkspaceId, String)>,
    pub config: WmConfig,
    next_id: WindowId,
}

impl<W: Clone + PartialEq> Default for Wm<W> {
    fn default() -> Self {
        Self::new(WmConfig::default())
    }
}

impl<W: Clone + PartialEq> Wm<W> {
    pub fn new(config: WmConfig) -> Self {
        Self {
            windows: Vec::new(),
            outputs: Vec::new(),
            focused_output: 0,
            focus: None,
            history: Vec::new(),
            scratch_visible: false,
            homes: Vec::new(),
            config,
            next_id: 1,
        }
    }

    // ---- outputs -------------------------------------------------------------------------

    pub fn add_output(&mut self, info: OutputInfo) {
        if let Some(o) = self.outputs.iter_mut().find(|o| o.name == info.name) {
            o.description = info.description;
            o.geometry = info.geometry;
            o.scale = info.scale;
            o.transform = info.transform;
            o.refresh_rate = info.refresh_rate;
            o.mode_px = info.mode_px;
            o.modes = info.modes;
            return;
        }
        // Show the lowest workspace not visible elsewhere, preferring one homed here.
        let homed = self
            .homes
            .iter()
            .find(|(ws, home)| home == &info.name && !self.is_visible_ws(*ws))
            .map(|(ws, _)| *ws);
        let active = homed.unwrap_or_else(|| {
            (1..=self.config.workspaces.max(1))
                .find(|ws| !self.is_visible_ws(*ws))
                .unwrap_or(1)
        });
        self.outputs.push(OutputSlot {
            name: info.name,
            description: info.description,
            geometry: info.geometry,
            tiled: Rect::default(),
            maximized: Rect::default(),
            active,
            scale: info.scale,
            transform: info.transform,
            refresh_rate: info.refresh_rate,
            mode_px: info.mode_px,
            modes: info.modes,
            dpms: true,
        });
        self.remember_home(active);
    }

    pub fn remove_output(&mut self, name: &str) {
        let Some(idx) = self.outputs.iter().position(|o| o.name == name) else {
            return;
        };
        self.outputs.remove(idx);
        if self.focused_output >= self.outputs.len() {
            self.focused_output = 0;
        }
        // Workspaces homed there now belong to the focused output.
        if let Some(target) = self
            .outputs
            .get(self.focused_output)
            .map(|o| o.name.clone())
        {
            for (_, home) in self.homes.iter_mut().filter(|(_, h)| h == name) {
                *home = target.clone();
            }
        }
    }

    pub fn set_dpms(&mut self, on: bool) {
        for o in &mut self.outputs {
            o.dpms = on;
        }
    }

    pub fn set_app_area(&mut self, output: &str, tiled: Rect, maximized: Rect) -> bool {
        match self.outputs.iter_mut().find(|o| o.name == output) {
            Some(o) => {
                o.tiled = tiled;
                o.maximized = maximized;
                true
            }
            None => false,
        }
    }

    pub fn focus_output_at(&mut self, x: i32, y: i32) {
        if let Some(idx) = self.outputs.iter().position(|o| {
            let g = o.geometry;
            x >= g.x && y >= g.y && x < g.x + g.w && y < g.y + g.h
        }) {
            self.focused_output = idx;
        }
    }

    pub fn focused_output_name(&self) -> Option<&str> {
        self.outputs
            .get(self.focused_output)
            .map(|o| o.name.as_str())
    }

    pub fn output_of_window(&self, id: WindowId) -> Option<&str> {
        let w = self.get(id)?;
        self.output_showing(w.workspace)
            .or_else(|| self.focused_output_name())
    }

    fn output_showing(&self, ws: WorkspaceId) -> Option<&str> {
        if ws == SCRATCH_WORKSPACE {
            return self.focused_output_name();
        }
        self.outputs
            .iter()
            .find(|o| o.active == ws)
            .map(|o| o.name.as_str())
    }

    fn is_visible_ws(&self, ws: WorkspaceId) -> bool {
        self.outputs.iter().any(|o| o.active == ws)
    }

    fn remember_home(&mut self, ws: WorkspaceId) {
        let Some(out) = self.outputs.iter().find(|o| o.active == ws) else {
            return;
        };
        let name = out.name.clone();
        match self.homes.iter_mut().find(|(w, _)| *w == ws) {
            Some(entry) => entry.1 = name,
            None => self.homes.push((ws, name)),
        }
    }

    // ---- windows -------------------------------------------------------------------------

    /// Manage a new window on the focused output's workspace and focus it.
    pub fn add_window(&mut self, handle: W, info: WindowInfo) -> WindowId {
        let id = self.next_id;
        self.next_id += 1;
        let workspace = self.current_workspace();
        let mode = if info.wants_float {
            WindowMode::Floating
        } else {
            WindowMode::Tiled
        };
        self.windows.push(Managed {
            id,
            handle,
            info,
            workspace,
            mode,
            restore_mode: mode,
            minimized: false,
            urgent: false,
            float_rect: None,
        });
        self.focus_window(id);
        id
    }

    pub fn remove_window(&mut self, id: WindowId) -> Option<W> {
        let idx = self.windows.iter().position(|w| w.id == id)?;
        let w = self.windows.remove(idx);
        self.history.retain(|h| *h != id);
        if self.focus == Some(id) {
            self.focus = None;
            self.refocus();
        }
        Some(w.handle)
    }

    pub fn id_of(&self, handle: &W) -> Option<WindowId> {
        self.windows
            .iter()
            .find(|w| &w.handle == handle)
            .map(|w| w.id)
    }

    pub fn handle(&self, id: WindowId) -> Option<&W> {
        self.get(id).map(|w| &w.handle)
    }

    pub fn handles(&self) -> impl Iterator<Item = (WindowId, &W)> {
        self.windows.iter().map(|w| (w.id, &w.handle))
    }

    fn get(&self, id: WindowId) -> Option<&Managed<W>> {
        self.windows.iter().find(|w| w.id == id)
    }

    fn get_mut(&mut self, id: WindowId) -> Option<&mut Managed<W>> {
        self.windows.iter_mut().find(|w| w.id == id)
    }

    pub fn set_title(&mut self, id: WindowId, title: &str) -> bool {
        match self.get_mut(id) {
            Some(w) if w.info.title != title => {
                w.info.title = title.to_string();
                true
            }
            _ => false,
        }
    }

    pub fn set_app_id(&mut self, id: WindowId, app_id: &str) -> bool {
        match self.get_mut(id) {
            Some(w) if w.info.app_id != app_id => {
                w.info.app_id = app_id.to_string();
                true
            }
            _ => false,
        }
    }

    pub fn set_urgent(&mut self, id: WindowId, urgent: bool) {
        if self.focus == Some(id) && urgent {
            return;
        }
        if let Some(w) = self.get_mut(id) {
            w.urgent = urgent;
        }
    }

    pub fn focused(&self) -> Option<WindowId> {
        self.focus
    }

    fn resolve(&self, id: Option<WindowId>) -> Option<WindowId> {
        id.or(self.focus).filter(|id| self.get(*id).is_some())
    }

    fn current_workspace(&self) -> WorkspaceId {
        self.outputs
            .get(self.focused_output)
            .map(|o| o.active)
            .unwrap_or(1)
    }

    fn is_shown(&self, w: &Managed<W>) -> bool {
        !w.minimized
            && if w.workspace == SCRATCH_WORKSPACE {
                self.scratch_visible
            } else {
                self.is_visible_ws(w.workspace)
            }
    }

    /// Focus a window, showing its workspace (and restoring it when minimized).
    pub fn focus_window(&mut self, id: WindowId) {
        let Some(w) = self.get(id) else {
            return;
        };
        if w.minimized {
            self.restore(id, None);
            return;
        }
        let ws = w.workspace;
        if ws == SCRATCH_WORKSPACE {
            self.scratch_visible = true;
        } else if let Some(idx) = self.outputs.iter().position(|o| o.active == ws) {
            self.focused_output = idx;
        } else {
            self.show_workspace(ws);
        }
        self.focus = Some(id);
        self.history.retain(|h| *h != id);
        self.history.push(id);
        if let Some(w) = self.get_mut(id) {
            w.urgent = false;
        }
    }

    /// Focus the most recently focused window that is still shown, if any.
    fn refocus(&mut self) {
        let next = self
            .history
            .iter()
            .rev()
            .copied()
            .find(|id| self.get(*id).is_some_and(|w| self.is_shown(w)))
            .or_else(|| {
                let ws = self.current_workspace();
                self.windows
                    .iter()
                    .find(|w| w.workspace == ws && self.is_shown(w))
                    .map(|w| w.id)
            });
        self.focus = next;
    }

    /// Clear keyboard focus from windows (the shell took it).
    pub fn unfocus(&mut self) {
        self.focus = None;
    }

    pub fn minimize(&mut self, id: Option<WindowId>) {
        let Some(id) = self.resolve(id) else {
            return;
        };
        if let Some(w) = self.get_mut(id) {
            w.minimized = true;
        }
        if self.focus == Some(id) {
            self.focus = None;
            self.refocus();
        }
    }

    pub fn restore(&mut self, id: WindowId, workspace: Option<WorkspaceId>) {
        let current = self.current_workspace();
        let Some(w) = self.get_mut(id) else {
            return;
        };
        w.minimized = false;
        // A window restored from its tab comes back where the user is looking.
        w.workspace = workspace.unwrap_or(current);
        self.focus_window(id);
    }

    pub fn close_target(&self, id: Option<WindowId>) -> Option<W> {
        self.resolve(id).and_then(|id| self.handle(id).cloned())
    }

    pub fn toggle_maximize(&mut self, id: Option<WindowId>) {
        self.toggle_mode(id, WindowMode::Maximized);
    }

    pub fn toggle_fullscreen(&mut self, id: Option<WindowId>) {
        self.toggle_mode(id, WindowMode::Fullscreen);
    }

    /// Apply a client's own (un)maximize or (un)fullscreen request.
    pub fn request_mode(&mut self, id: WindowId, mode: WindowMode, on: bool) {
        let Some(w) = self.get_mut(id) else {
            return;
        };
        if on && w.mode != mode {
            if !matches!(w.mode, WindowMode::Maximized | WindowMode::Fullscreen) {
                w.restore_mode = w.mode;
            }
            w.mode = mode;
        } else if !on && w.mode == mode {
            w.mode = w.restore_mode;
        }
    }

    fn toggle_mode(&mut self, id: Option<WindowId>, mode: WindowMode) {
        let Some(id) = self.resolve(id) else {
            return;
        };
        let on = self.get(id).is_some_and(|w| w.mode != mode);
        self.request_mode(id, mode, on);
        self.focus_window(id);
    }

    pub fn toggle_float(&mut self, id: Option<WindowId>, current: Option<Rect>) {
        let Some(id) = self.resolve(id) else {
            return;
        };
        let area = self.tiled_area_for(id);
        let Some(w) = self.get_mut(id) else {
            return;
        };
        w.mode = match w.mode {
            WindowMode::Floating => WindowMode::Tiled,
            _ => {
                if w.float_rect.is_none() {
                    w.float_rect = Some(centered(area, current, w.info.preferred));
                }
                WindowMode::Floating
            }
        };
        w.restore_mode = w.mode;
    }

    /// Remember a floating window's position after an interactive move or resize.
    pub fn set_float_rect(&mut self, id: WindowId, rect: Rect) {
        if let Some(w) = self.get_mut(id) {
            if w.mode == WindowMode::Floating {
                w.float_rect = Some(rect);
            }
        }
    }

    pub fn is_floating(&self, id: WindowId) -> bool {
        self.get(id).is_some_and(|w| w.mode == WindowMode::Floating)
    }

    pub fn move_to_workspace(
        &mut self,
        id: Option<WindowId>,
        target: WorkspaceTarget,
        follow: bool,
    ) {
        let Some(id) = self.resolve(id) else {
            return;
        };
        let Some(ws) = self.resolve_workspace(target) else {
            return;
        };
        if let Some(w) = self.get_mut(id) {
            w.workspace = ws;
            w.minimized = false;
        }
        if follow {
            self.focus_workspace(WorkspaceTarget::Id(ws));
            self.focus_window(id);
        } else if self.focus == Some(id) {
            self.focus = None;
            self.refocus();
        }
    }

    pub fn toggle_scratch(&mut self) {
        self.scratch_visible = !self.scratch_visible;
        if self.scratch_visible {
            if let Some(id) = self
                .windows
                .iter()
                .rev()
                .find(|w| w.workspace == SCRATCH_WORKSPACE && !w.minimized)
                .map(|w| w.id)
            {
                self.focus_window(id);
            }
        } else if self
            .focus
            .and_then(|f| self.get(f))
            .is_some_and(|w| w.workspace == SCRATCH_WORKSPACE)
        {
            self.focus = None;
            self.refocus();
        }
    }

    fn resolve_workspace(&self, target: WorkspaceTarget) -> Option<WorkspaceId> {
        let n = self.config.workspaces.max(1);
        let current = self.current_workspace().clamp(1, n);
        Some(match target {
            WorkspaceTarget::Id(id) if id == SCRATCH_WORKSPACE => SCRATCH_WORKSPACE,
            WorkspaceTarget::Id(id) if (1..=n).contains(&id) => id,
            WorkspaceTarget::Id(_) => return None,
            WorkspaceTarget::Next => current % n + 1,
            WorkspaceTarget::Prev => (current + n - 2) % n + 1,
            WorkspaceTarget::Empty => (1..=n).find(|ws| {
                !self.is_visible_ws(*ws) && !self.windows.iter().any(|w| w.workspace == *ws)
            })?,
        })
    }

    /// Show a workspace on the focused output, or focus the output already showing it.
    pub fn focus_workspace(&mut self, target: WorkspaceTarget) {
        let Some(ws) = self.resolve_workspace(target) else {
            return;
        };
        if ws == SCRATCH_WORKSPACE {
            self.toggle_scratch();
            return;
        }
        if let Some(idx) = self.outputs.iter().position(|o| o.active == ws) {
            self.focused_output = idx;
        } else {
            self.show_workspace(ws);
        }
        self.focus = None;
        let shown_ws = ws;
        let last = self.history.iter().rev().copied().find(|id| {
            self.get(*id)
                .is_some_and(|w| w.workspace == shown_ws && !w.minimized)
        });
        self.focus = last.or_else(|| {
            self.windows
                .iter()
                .find(|w| w.workspace == shown_ws && !w.minimized)
                .map(|w| w.id)
        });
    }

    fn show_workspace(&mut self, ws: WorkspaceId) {
        if let Some(o) = self.outputs.get_mut(self.focused_output) {
            o.active = ws;
        }
        self.remember_home(ws);
    }

    /// Visible windows of the focused output, in tiling order.
    fn visible_on_focused(&self) -> Vec<WindowId> {
        let ws = self.current_workspace();
        self.windows
            .iter()
            .filter(|w| w.workspace == ws && !w.minimized)
            .map(|w| w.id)
            .collect()
    }

    pub fn cycle_focus(&mut self, reverse: bool) {
        let ids = self.visible_on_focused();
        if ids.is_empty() {
            return;
        }
        let pos = self
            .focus
            .and_then(|f| ids.iter().position(|i| *i == f))
            .unwrap_or(0);
        let next = if reverse {
            (pos + ids.len() - 1) % ids.len()
        } else {
            (pos + 1) % ids.len()
        };
        self.focus_window(ids[next]);
    }

    fn neighbour(&self, direction: Direction) -> Option<WindowId> {
        let focus = self.focus?;
        let placements = self.arrange();
        let from = placements.iter().find(|p| p.id == focus)?.rect;
        let (fx, fy) = center(from);
        placements
            .iter()
            .filter(|p| p.visible && p.id != focus)
            .filter_map(|p| {
                let (x, y) = center(p.rect);
                let (dx, dy) = (x - fx, y - fy);
                let ahead = match direction {
                    Direction::Left => dx < 0 && dx.abs() >= dy.abs() / 2,
                    Direction::Right => dx > 0 && dx.abs() >= dy.abs() / 2,
                    Direction::Up => dy < 0 && dy.abs() >= dx.abs() / 2,
                    Direction::Down => dy > 0 && dy.abs() >= dx.abs() / 2,
                };
                ahead.then_some((p.id, dx * dx + dy * dy))
            })
            .min_by_key(|(_, d)| *d)
            .map(|(id, _)| id)
    }

    pub fn focus_direction(&mut self, direction: Direction) {
        if let Some(id) = self.neighbour(direction) {
            self.focus_window(id);
        }
    }

    /// Swap the focused window with its neighbour in the tiling order.
    pub fn move_direction(&mut self, direction: Direction) {
        let (Some(focus), Some(other)) = (self.focus, self.neighbour(direction)) else {
            return;
        };
        let a = self.windows.iter().position(|w| w.id == focus);
        let b = self.windows.iter().position(|w| w.id == other);
        if let (Some(a), Some(b)) = (a, b) {
            self.windows.swap(a, b);
        }
    }

    // ---- layout --------------------------------------------------------------------------

    fn tiled_area_for(&self, id: WindowId) -> Rect {
        let name = self.output_of_window(id).map(str::to_string);
        self.outputs
            .iter()
            .find(|o| Some(&o.name) == name.as_ref())
            .map(|o| area(o.geometry, o.tiled))
            .unwrap_or_default()
    }

    /// Where every window goes. Hidden windows get `visible: false`.
    pub fn arrange(&self) -> Vec<Placement<W>> {
        let mut out = Vec::with_capacity(self.windows.len());
        for (idx, o) in self.outputs.iter().enumerate() {
            let tiled_area = inset(area(o.geometry, o.tiled), self.config.gaps_out);
            let max_area = area(o.geometry, o.maximized);
            let on_ws: Vec<&Managed<W>> = self
                .windows
                .iter()
                .filter(|w| !w.minimized && w.workspace == o.active)
                .collect();
            let tiled: Vec<&&Managed<W>> = on_ws
                .iter()
                .filter(|w| w.mode == WindowMode::Tiled)
                .collect();
            let rects = tile(
                tiled_area,
                tiled.len(),
                self.config.layout,
                self.config.gaps_in,
            );
            for (w, rect) in tiled.iter().zip(rects) {
                out.push(self.placement(w, rect, true));
            }
            for w in on_ws.iter().filter(|w| w.mode != WindowMode::Tiled) {
                let rect = match w.mode {
                    WindowMode::Maximized => max_area,
                    WindowMode::Fullscreen => o.geometry,
                    _ => w.float_rect.unwrap_or_else(|| {
                        centered(area(o.geometry, o.tiled), None, w.info.preferred)
                    }),
                };
                out.push(self.placement(w, rect, true));
            }
            if idx == self.focused_output && self.scratch_visible {
                for w in self
                    .windows
                    .iter()
                    .filter(|w| !w.minimized && w.workspace == SCRATCH_WORKSPACE)
                {
                    let rect = w.float_rect.unwrap_or_else(|| {
                        centered(area(o.geometry, o.tiled), None, w.info.preferred)
                    });
                    let mut p = self.placement(w, rect, true);
                    p.mode = WindowMode::Floating;
                    p.on_top = true;
                    out.push(p);
                }
            }
        }
        for w in &self.windows {
            if !out.iter().any(|p| p.id == w.id) {
                out.push(self.placement(w, Rect::default(), false));
            }
        }
        out
    }

    fn placement(&self, w: &Managed<W>, rect: Rect, visible: bool) -> Placement<W> {
        Placement {
            id: w.id,
            handle: w.handle.clone(),
            rect,
            visible,
            mode: w.mode,
            activated: self.focus == Some(w.id),
            tiled: w.mode == WindowMode::Tiled,
            on_top: matches!(w.mode, WindowMode::Fullscreen | WindowMode::Floating),
        }
    }

    // ---- protocol snapshots --------------------------------------------------------------

    pub fn windows(&self) -> Vec<Window> {
        self.windows
            .iter()
            .map(|w| Window {
                id: w.id,
                app_id: w.info.app_id.clone(),
                title: w.info.title.clone(),
                workspace: w.workspace,
                mode: w.mode,
                minimized: w.minimized,
                urgent: w.urgent,
                xwayland: w.info.xwayland,
                pid: w.info.pid,
            })
            .collect()
    }

    pub fn workspaces(&self) -> Vec<Workspace> {
        let mut ids: Vec<WorkspaceId> = self.outputs.iter().map(|o| o.active).collect();
        ids.extend(self.windows.iter().map(|w| w.workspace));
        if self.windows.iter().any(|w| w.minimized) {
            ids.push(MINIMIZED_WORKSPACE);
        }
        ids.sort_unstable();
        ids.dedup();
        ids.into_iter()
            .map(|id| {
                let members = self.windows.iter().filter(|w| {
                    if id == MINIMIZED_WORKSPACE {
                        w.minimized
                    } else {
                        w.workspace == id && !w.minimized
                    }
                });
                let (count, fullscreen) = members.fold((0u32, false), |(n, f), w| {
                    (n + 1, f || w.mode == WindowMode::Fullscreen)
                });
                Workspace {
                    id,
                    name: match id {
                        MINIMIZED_WORKSPACE => "minimized".into(),
                        SCRATCH_WORKSPACE => "scratch".into(),
                        n => n.to_string(),
                    },
                    monitor: self
                        .output_showing(id)
                        .map(str::to_string)
                        .or_else(|| {
                            self.homes
                                .iter()
                                .find(|(w, _)| *w == id)
                                .map(|(_, h)| h.clone())
                        })
                        .or_else(|| self.focused_output_name().map(str::to_string))
                        .unwrap_or_default(),
                    windows: count,
                    has_fullscreen: fullscreen,
                }
            })
            .collect()
    }

    pub fn monitors(&self) -> Vec<Monitor> {
        self.outputs
            .iter()
            .enumerate()
            .map(|(i, o)| Monitor {
                name: o.name.clone(),
                description: o.description.clone(),
                width: o.mode_px.0,
                height: o.mode_px.1,
                refresh_rate: o.refresh_rate,
                x: o.geometry.x,
                y: o.geometry.y,
                scale: o.scale,
                transform: o.transform,
                focused: i == self.focused_output,
                active_workspace: o.active,
                dpms: o.dpms,
                disabled: false,
                modes: o.modes.clone(),
            })
            .collect()
    }
}

/// `area` relative to the output, or the whole output when it is empty.
fn area(output: Rect, rel: Rect) -> Rect {
    if rel.is_empty() {
        output
    } else {
        Rect::new(output.x + rel.x, output.y + rel.y, rel.w, rel.h)
    }
}

fn inset(r: Rect, by: i32) -> Rect {
    if r.w <= 2 * by || r.h <= 2 * by {
        return r;
    }
    Rect::new(r.x + by, r.y + by, r.w - 2 * by, r.h - 2 * by)
}

fn center(r: Rect) -> (i32, i32) {
    (r.x + r.w / 2, r.y + r.h / 2)
}

/// A floating rectangle centered in `area`: the current size, else the client's preferred one,
/// else 60% of the area.
fn centered(area: Rect, current: Option<Rect>, preferred: Option<(i32, i32)>) -> Rect {
    let (w, h) = current
        .map(|r| (r.w, r.h))
        .filter(|(w, h)| *w > 0 && *h > 0)
        .or(preferred.filter(|(w, h)| *w > 0 && *h > 0))
        .unwrap_or((area.w * 3 / 5, area.h * 3 / 5));
    let (w, h) = (w.min(area.w).max(1), h.min(area.h).max(1));
    Rect::new(area.x + (area.w - w) / 2, area.y + (area.h - h) / 2, w, h)
}

/// Split `area` into `n` tiles separated by `gap`.
pub fn tile(area: Rect, n: usize, layout: TileLayout, gap: i32) -> Vec<Rect> {
    if n == 0 {
        return Vec::new();
    }
    if n == 1 {
        return vec![area];
    }
    match layout {
        TileLayout::Dwindle => {
            let mut out = Vec::with_capacity(n);
            let mut rest = area;
            for i in 0..n {
                if i == n - 1 {
                    out.push(rest);
                    break;
                }
                let (a, b) = split(rest, rest.w >= rest.h, gap);
                out.push(a);
                rest = b;
            }
            out
        }
        TileLayout::Master => {
            let (master, stack) = split(area, true, gap);
            let mut out = vec![master];
            let k = (n - 1) as i32;
            let total_gap = gap * (k - 1);
            let h = (stack.h - total_gap) / k;
            for i in 0..k {
                let y = stack.y + i * (h + gap);
                let hh = if i == k - 1 { stack.y + stack.h - y } else { h };
                out.push(Rect::new(stack.x, y, stack.w, hh));
            }
            out
        }
    }
}

/// Halve `r` side by side (`vertical` divider) or one above the other.
fn split(r: Rect, vertical: bool, gap: i32) -> (Rect, Rect) {
    if vertical {
        let a = (r.w - gap) / 2;
        (
            Rect::new(r.x, r.y, a, r.h),
            Rect::new(r.x + a + gap, r.y, r.w - a - gap, r.h),
        )
    } else {
        let a = (r.h - gap) / 2;
        (
            Rect::new(r.x, r.y, r.w, a),
            Rect::new(r.x, r.y + a + gap, r.w, r.h - a - gap),
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn wm() -> Wm<&'static str> {
        let mut wm = Wm::new(WmConfig {
            gaps_in: 0,
            gaps_out: 0,
            layout: TileLayout::Dwindle,
            workspaces: 10,
        });
        wm.add_output(OutputInfo {
            name: "DP-1".into(),
            geometry: Rect::new(0, 0, 1920, 1080),
            scale: 1.0,
            ..Default::default()
        });
        wm
    }

    fn info() -> WindowInfo {
        WindowInfo::default()
    }

    fn placed<'a>(wm: &Wm<&'a str>, h: &str) -> Placement<&'a str> {
        wm.arrange().into_iter().find(|p| p.handle == h).unwrap()
    }

    #[test]
    fn tiles_fill_the_app_area() {
        let mut wm = wm();
        wm.set_app_area(
            "DP-1",
            Rect::new(400, 40, 1120, 800),
            Rect::new(0, 40, 1920, 800),
        );
        wm.add_window("a", info());
        assert_eq!(placed(&wm, "a").rect, Rect::new(400, 40, 1120, 800));
        wm.add_window("b", info());
        assert_eq!(placed(&wm, "a").rect, Rect::new(400, 40, 560, 800));
        assert_eq!(placed(&wm, "b").rect, Rect::new(960, 40, 560, 800));
        // The third window splits the second one along its longer (vertical) side.
        wm.add_window("c", info());
        assert_eq!(placed(&wm, "b").rect, Rect::new(960, 40, 560, 400));
        assert_eq!(placed(&wm, "c").rect, Rect::new(960, 440, 560, 400));
        assert!(placed(&wm, "c").activated);
    }

    #[test]
    fn master_layout_and_gaps() {
        let r = tile(Rect::new(0, 0, 1000, 600), 3, TileLayout::Master, 10);
        assert_eq!(r[0], Rect::new(0, 0, 495, 600));
        assert_eq!(r[1], Rect::new(505, 0, 495, 295));
        assert_eq!(r[2], Rect::new(505, 305, 495, 295));
    }

    #[test]
    fn minimize_restore_and_focus() {
        let mut wm = wm();
        let a = wm.add_window("a", info());
        let b = wm.add_window("b", info());
        assert_eq!(wm.focused(), Some(b));
        wm.minimize(None);
        assert_eq!(wm.focused(), Some(a));
        assert!(!placed(&wm, "b").visible);
        // The lone visible window takes the whole area again.
        assert_eq!(placed(&wm, "a").rect, Rect::new(0, 0, 1920, 1080));
        let ws = wm.workspaces();
        assert!(ws
            .iter()
            .any(|w| w.id == MINIMIZED_WORKSPACE && w.windows == 1));
        // Restoring brings it back to the current workspace, focused.
        wm.focus_workspace(WorkspaceTarget::Id(3));
        wm.restore(b, None);
        assert_eq!(
            wm.windows().iter().find(|w| w.id == b).unwrap().workspace,
            3
        );
        assert_eq!(wm.focused(), Some(b));
        assert!(placed(&wm, "b").visible && !placed(&wm, "a").visible);
    }

    #[test]
    fn maximize_uses_the_shells_maximized_area() {
        let mut wm = wm();
        wm.set_app_area(
            "DP-1",
            Rect::new(400, 40, 1120, 800),
            Rect::new(0, 40, 1920, 800),
        );
        wm.add_window("a", info());
        wm.add_window("b", info());
        wm.toggle_maximize(None);
        let b = placed(&wm, "b");
        assert_eq!(b.mode, WindowMode::Maximized);
        assert_eq!(b.rect, Rect::new(0, 40, 1920, 800));
        assert_eq!(placed(&wm, "a").rect, Rect::new(400, 40, 1120, 800));
        wm.toggle_maximize(None);
        assert_eq!(placed(&wm, "b").mode, WindowMode::Tiled);
        wm.toggle_fullscreen(None);
        assert_eq!(placed(&wm, "b").rect, Rect::new(0, 0, 1920, 1080));
        assert!(placed(&wm, "b").on_top);
        wm.request_mode(2, WindowMode::Fullscreen, false);
        assert_eq!(placed(&wm, "b").mode, WindowMode::Tiled);
    }

    #[test]
    fn workspaces_follow_outputs() {
        let mut wm = wm();
        wm.add_output(OutputInfo {
            name: "HDMI-A-1".into(),
            geometry: Rect::new(1920, 0, 1280, 1024),
            scale: 1.0,
            ..Default::default()
        });
        let mons = wm.monitors();
        assert_eq!(mons[0].active_workspace, 1);
        assert_eq!(mons[1].active_workspace, 2);
        let a = wm.add_window("a", info());
        wm.move_to_workspace(Some(a), WorkspaceTarget::Id(2), false);
        // Workspace 2 is visible on the second output: the window shows there.
        let p = placed(&wm, "a");
        assert!(p.visible);
        assert_eq!(p.rect, Rect::new(1920, 0, 1280, 1024));
        // Focusing workspace 2 focuses the output showing it.
        wm.focus_workspace(WorkspaceTarget::Id(2));
        assert_eq!(wm.focused_output_name(), Some("HDMI-A-1"));
        assert_eq!(wm.focused(), Some(a));
        wm.focus_workspace(WorkspaceTarget::Next);
        assert_eq!(wm.monitors()[1].active_workspace, 3);
        assert!(!placed(&wm, "a").visible);
        wm.remove_output("HDMI-A-1");
        assert_eq!(wm.focused_output_name(), Some("DP-1"));
        wm.focus_window(a);
        assert_eq!(wm.monitors()[0].active_workspace, 2);
        assert!(placed(&wm, "a").visible);
    }

    #[test]
    fn floating_and_scratch() {
        let mut wm = wm();
        let a = wm.add_window(
            "a",
            WindowInfo {
                wants_float: true,
                preferred: Some((400, 300)),
                ..Default::default()
            },
        );
        assert_eq!(placed(&wm, "a").rect, Rect::new(760, 390, 400, 300));
        wm.toggle_float(Some(a), None);
        assert_eq!(placed(&wm, "a").mode, WindowMode::Tiled);
        wm.move_to_workspace(Some(a), WorkspaceTarget::Id(SCRATCH_WORKSPACE), false);
        assert!(!placed(&wm, "a").visible);
        assert_eq!(wm.focused(), None);
        wm.toggle_scratch();
        assert!(placed(&wm, "a").visible && placed(&wm, "a").on_top);
        assert_eq!(wm.focused(), Some(a));
        wm.toggle_scratch();
        assert!(!placed(&wm, "a").visible);
        assert_eq!(wm.focused(), None);
    }

    #[test]
    fn directional_focus_and_swap() {
        let mut wm = wm();
        let a = wm.add_window("a", info());
        let b = wm.add_window("b", info());
        let c = wm.add_window("c", info());
        // a | b over c
        wm.focus_direction(Direction::Up);
        assert_eq!(wm.focused(), Some(b));
        wm.focus_direction(Direction::Left);
        assert_eq!(wm.focused(), Some(a));
        wm.move_direction(Direction::Right);
        assert_eq!(placed(&wm, "a").rect, Rect::new(960, 0, 960, 540));
        wm.cycle_focus(false);
        assert_eq!(wm.focused(), Some(c));
        wm.remove_window(c);
        assert_eq!(wm.focused(), Some(a));
        assert_eq!(wm.workspaces()[0].windows, 2);
        let _ = b;
    }
}
