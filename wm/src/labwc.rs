//! labwc (and other wlroots compositors) through `zwlr_foreign_toplevel_management_v1`.
//!
//! labwc stacks windows; eDEX's labwc configuration (see `settings::labwc_export`) maximizes
//! every normal window, and labwc fits maximized windows into the area the shell's reserver
//! surfaces leave free: the terminal slot. So here:
//!
//! * a maximized window is *tiled* (it covers the terminal); one the user un-maximized is
//!   *floating*;
//! * eDEX's own maximize (the □ control: full width, side panels hidden) is a flag kept here, as
//!   labwc has no such state; the shell then shrinks the side reservers and labwc refits the
//!   window;
//! * there is one workspace;
//! * reconfigure and exit go to the compositor named by `LABWC_PID` (SIGHUP / SIGTERM).

use std::collections::BTreeSet;

use anyhow::{anyhow, Context, Result};
use platform::Toplevels;

use crate::{Capabilities, Update, WindowManager, WmKind, WmOutput, WmState, WmWindow};

pub struct Labwc {
    toplevels: Toplevels,
    /// Windows the user gave the full width (eDEX maximize).
    wide: BTreeSet<u64>,
    state: WmState,
    /// The compositor is labwc (`LABWC_PID` is set), not another wlroots compositor.
    labwc_pid: Option<i32>,
    /// The state changed here (eDEX maximize), not through a compositor event.
    local_change: bool,
}

impl Labwc {
    pub fn new(toplevels: Toplevels) -> Self {
        let labwc_pid = std::env::var("LABWC_PID")
            .ok()
            .and_then(|p| p.trim().parse().ok())
            .filter(|p: &i32| *p > 0);
        let mut l = Self {
            toplevels,
            wide: BTreeSet::new(),
            state: WmState::default(),
            labwc_pid,
            local_change: false,
        };
        l.rebuild(&[]);
        l
    }

    pub fn is_labwc(&self) -> bool {
        self.labwc_pid.is_some()
    }

    /// Rebuild the generic state from the compositor's list. `outputs` are the shell's outputs
    /// (the focused one first), as labwc reports no focus per output.
    fn rebuild(&mut self, outputs: &[String]) {
        let list = self.toplevels.list();
        self.wide.retain(|id| list.iter().any(|t| t.id == *id));
        let windows: Vec<WmWindow> = list
            .iter()
            .filter(|t| t.parent.is_none())
            .map(|t| WmWindow {
                id: t.id.to_string(),
                class: t.app_id.clone(),
                title: t.title.clone(),
                workspace: None,
                outputs: t.outputs.clone(),
                // labwc stacks windows and eDEX's rc.xml maximizes them into the slot; other
                // wlroots compositors (sway) tile windows without reporting it.
                floating: self.labwc_pid.is_some() && !t.maximized && !t.fullscreen,
                maximized: self.wide.contains(&t.id),
                fullscreen: t.fullscreen,
                minimized: t.minimized,
                active: t.activated,
            })
            .collect();
        let active_window = windows
            .iter()
            .find(|w| w.active && !w.minimized)
            .map(|w| (w.class.clone(), w.title.clone()));
        let mut outs: Vec<WmOutput> = outputs
            .iter()
            .enumerate()
            .map(|(i, n)| WmOutput {
                name: n.clone(),
                focused: i == 0,
                active_workspace: None,
            })
            .collect();
        if outs.is_empty() {
            outs = self.state.outputs.clone();
        }
        self.state = WmState {
            connected: self.toplevels.active(),
            name: if self.labwc_pid.is_some() {
                "labwc".into()
            } else {
                "wlr-foreign-toplevel".into()
            },
            version: std::env::var("LABWC_VER").unwrap_or_default(),
            outputs: outs,
            workspaces: Vec::new(),
            windows,
            active_window,
            keyboard_layout: self.state.keyboard_layout.clone(),
        };
    }

    fn id(id: &str) -> Result<u64> {
        id.parse().map_err(|_| anyhow!("bad window id {id}"))
    }

    fn signal(&self, sig: libc::c_int) -> Result<()> {
        let pid = self
            .labwc_pid
            .ok_or_else(|| anyhow!("LABWC_PID is not set: not running under labwc"))?;
        if unsafe { libc::kill(pid, sig) } != 0 {
            return Err(std::io::Error::last_os_error()).context("signalling labwc");
        }
        Ok(())
    }

    fn known(&self, ok: bool, id: &str) -> Result<()> {
        if ok {
            Ok(())
        } else {
            Err(anyhow!("no window {id}"))
        }
    }
}

impl WindowManager for Labwc {
    fn kind(&self) -> WmKind {
        WmKind::Labwc
    }

    fn state(&self) -> &WmState {
        &self.state
    }

    fn refresh(&mut self) -> Update {
        let before = self.state.clone();
        let outputs: Vec<String> = before.outputs.iter().map(|o| o.name.clone()).collect();
        self.rebuild(&outputs);
        Update {
            changed: self.state != before || std::mem::take(&mut self.local_change),
            bell: false,
            lost: before.connected && !self.state.connected,
        }
    }

    fn set_outputs(&mut self, outputs: &[String]) {
        if self
            .state
            .outputs
            .iter()
            .map(|o| &o.name)
            .ne(outputs.iter())
        {
            self.rebuild(outputs);
        }
    }

    fn focus_window(&mut self, id: &str) -> Result<()> {
        let ok = self.toplevels.activate(Self::id(id)?);
        self.known(ok, id)
    }

    fn minimize(&mut self, id: &str) -> Result<()> {
        let ok = self.toplevels.set_minimized(Self::id(id)?, true);
        self.known(ok, id)
    }

    fn restore(&mut self, id: &str) -> Result<()> {
        let n = Self::id(id)?;
        let ok = self.toplevels.set_minimized(n, false) && self.toplevels.activate(n);
        self.known(ok, id)
    }

    fn toggle_maximize(&mut self, id: &str) -> Result<()> {
        let n = Self::id(id)?;
        let tiled = self
            .toplevels
            .list()
            .iter()
            .find(|t| t.id == n)
            .map(|t| t.maximized)
            .ok_or_else(|| anyhow!("no window {id}"))?;
        if self.labwc_pid.is_some() && !tiled {
            // A floating window first goes back into the slot.
            self.toplevels.set_maximized(n, true);
            self.wide.insert(n);
        } else if !self.wide.remove(&n) {
            self.wide.insert(n);
        }
        self.toplevels.activate(n);
        let outputs: Vec<String> = self.state.outputs.iter().map(|o| o.name.clone()).collect();
        self.rebuild(&outputs);
        self.local_change = true;
        Ok(())
    }

    fn close(&mut self, id: &str) -> Result<()> {
        let ok = self.toplevels.close(Self::id(id)?);
        self.known(ok, id)
    }

    fn reveal_shell(&mut self) -> Result<bool> {
        // One workspace: the shell minimizes what covers the terminal itself.
        Ok(false)
    }

    fn exit(&self) -> Result<()> {
        self.signal(libc::SIGTERM)
    }

    fn export(&self, config: &settings::Config) -> Result<()> {
        settings::labwc_export::export(config, &settings::labwc_export::config_dir())
    }

    fn reload(&self) -> Result<()> {
        // `labwc --reconfigure` sends the same signal.
        self.signal(libc::SIGHUP)
    }

    fn capabilities(&self) -> Capabilities {
        Capabilities {
            gaps_outer: true,
            rounding: true,
            input: true,
            ..Capabilities::default()
        }
    }
}
