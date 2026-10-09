//! Interaction logic for the overlays: launcher, power menu, notification history, and the
//! settings/privacy forms (whose contents live in `forms`).

use platform::{button, KeyInput, Platform};
use ui::{
    hit::HitTarget,
    overlays::{
        launcher::VISIBLE_RESULTS,
        notifications::{HIT_CLEAR_ALL, HIT_TOGGLE_DND},
    },
    state::{LauncherResult, NotificationRow, OverlayKind, PowerAction},
};
use xkbcommon::xkb::keysyms as ks;

use crate::{app::App, events::AppEvent};

/// Called right before an overlay surface is created.
pub fn prepare(app: &mut App, kind: OverlayKind) {
    match kind {
        OverlayKind::Launcher => {
            app.state.launcher.reset();
            app.apps = launcher::scan_applications();
            refresh_launcher(app);
        }
        OverlayKind::Power => {
            app.state.power.selected = 0;
            app.state.power.confirm = None;
            let p = &app.sys.power;
            let caps = &app.caps;
            let mut available = Vec::new();
            if caps.lock {
                available.push(PowerAction::Lock);
            }
            available.push(PowerAction::Logout);
            if caps.suspend && (p.can_suspend || !app.sys.power.profiles.is_empty()) {
                available.push(PowerAction::Suspend);
            }
            if caps.hibernate && p.can_hibernate {
                available.push(PowerAction::Hibernate);
            }
            available.push(PowerAction::Reboot);
            available.push(PowerAction::PowerOff);
            app.state.power.available = available;
        }
        OverlayKind::Notifications => {
            app.store.mark_read();
            refresh_notifications(app);
            app.state.status.unread_notifications = 0;
        }
        OverlayKind::Settings => crate::forms::settings::open(app),
        OverlayKind::Privacy => crate::forms::privacy::open(app),
    }
}

pub fn refresh_launcher(app: &mut App) {
    let results = app.search.search(
        &app.state.launcher.query,
        &app.apps,
        &app.launch_history,
        40,
    );
    app.state.launcher.results = results
        .iter()
        .map(|a| LauncherResult {
            name: a.name.clone(),
            comment: a.comment.clone(),
            category: a.category(),
        })
        .collect();
    let l = &mut app.state.launcher;
    if l.selected >= l.results.len() {
        l.selected = l.results.len().saturating_sub(1);
    }
    if l.selected < l.scroll {
        l.scroll = l.selected;
    } else if l.selected >= l.scroll + VISIBLE_RESULTS {
        l.scroll = l.selected + 1 - VISIBLE_RESULTS;
    }
}

pub fn refresh_notifications(app: &mut App) {
    let rows: Vec<NotificationRow> = app
        .store
        .history
        .iter()
        .map(|h| NotificationRow {
            id: h.id,
            app: h.app.clone(),
            summary: h.summary.clone(),
            body: h.body.clone(),
            time: chrono::DateTime::from_timestamp(h.time as i64, 0)
                .map(|t| t.with_timezone(&chrono::Local).format("%H:%M").to_string())
                .unwrap_or_default(),
            urgency: h.urgency as u8,
        })
        .collect();
    let v = &mut app.state.notifications;
    v.rows = rows;
    v.dnd = app.store.dnd;
    if v.selected >= v.rows.len() {
        v.selected = v.rows.len().saturating_sub(1);
    }
}

fn launch_selected(app: &mut App, platform: &mut Platform<AppEvent>, force_terminal: bool) {
    let idx = app.state.launcher.selected;
    let results = app.search.search(
        &app.state.launcher.query,
        &app.apps,
        &app.launch_history,
        40,
    );
    let Some(entry) = results.get(idx).cloned().cloned() else {
        // No app: run the query as a shell command in a terminal tab.
        let q = app.state.launcher.query.trim().to_string();
        if !q.is_empty() {
            app.terminal.write_input(format!("{q}\n").as_bytes());
        }
        app.close_overlay(platform);
        return;
    };
    let opts = launcher::LaunchOptions {
        terminal_command: app.config.launcher.terminal_command.clone(),
        force_terminal,
    };
    let cmd = launcher::runner::command_line(&entry, &opts);
    tracing::info!(app = %entry.id, %cmd, "launching");
    app.spawn(&cmd);
    app.launch_history.record(&entry.id);
    app.close_overlay(platform);
}

fn run_power(app: &mut App, platform: &mut Platform<AppEvent>, action: PowerAction) {
    use system::LogindAction as L;
    app.close_overlay(platform);
    match action {
        PowerAction::Lock => {
            app.spawn("hyprlock || loginctl lock-session");
        }
        PowerAction::Logout => {
            if let Err(e) = app.wm.exit() {
                tracing::info!("window manager logout: {e:#}; ending the session");
                let _ = launcher::runner::spawn_detached(
                    "loginctl terminate-session \"$XDG_SESSION_ID\"",
                );
                app.quit = true;
            }
        }
        PowerAction::Suspend => app.system.send(system::SysRequest::Logind(L::Suspend)),
        PowerAction::Hibernate => app.system.send(system::SysRequest::Logind(L::Hibernate)),
        PowerAction::Reboot => app.system.send(system::SysRequest::Logind(L::Reboot)),
        PowerAction::PowerOff => app.system.send(system::SysRequest::Logind(L::PowerOff)),
    }
}

fn power_activate(app: &mut App, platform: &mut Platform<AppEvent>, idx: usize) {
    let Some(&action) = app.state.power.available.get(idx) else {
        return;
    };
    let needs_confirm = matches!(
        action,
        PowerAction::Reboot | PowerAction::PowerOff | PowerAction::Logout
    );
    if needs_confirm && app.state.power.confirm != Some(action) {
        app.state.power.confirm = Some(action);
        return;
    }
    run_power(app, platform, action);
}

pub fn key(app: &mut App, platform: &mut Platform<AppEvent>, key: &KeyInput) {
    let Some(kind) = app.state.overlay else {
        return;
    };
    if key.keysym == ks::KEY_Escape {
        match kind {
            OverlayKind::Settings if app.state.settings.form_state.editing.is_some() => {
                app.state.settings.form_state.editing = None;
                crate::forms::settings::rebuild(app);
            }
            OverlayKind::Privacy if app.state.privacy.form_state.editing.is_some() => {
                app.state.privacy.form_state.editing = None;
                crate::forms::privacy::rebuild(app);
            }
            OverlayKind::Power if app.state.power.confirm.is_some() => {
                app.state.power.confirm = None
            }
            _ => app.close_overlay(platform),
        }
        return;
    }
    match kind {
        OverlayKind::Launcher => {
            let l = &mut app.state.launcher;
            match key.keysym {
                ks::KEY_Up => l.move_up(),
                ks::KEY_Down => l.move_down(),
                ks::KEY_Page_Up => l.selected = l.selected.saturating_sub(VISIBLE_RESULTS),
                ks::KEY_Page_Down => {
                    l.selected =
                        (l.selected + VISIBLE_RESULTS).min(l.results.len().saturating_sub(1))
                }
                ks::KEY_Return | ks::KEY_KP_Enter => {
                    let force = key.modifiers.ctrl;
                    launch_selected(app, platform, force);
                    return;
                }
                ks::KEY_BackSpace => {
                    if key.modifiers.ctrl {
                        l.query.clear();
                    } else {
                        l.query.pop();
                    }
                    l.selected = 0;
                }
                ks::KEY_Tab => l.move_down(),
                _ => {
                    if let Some(t) = key.text.as_deref() {
                        if !key.modifiers.ctrl
                            && !key.modifiers.alt
                            && !t.chars().any(|c| c.is_control())
                        {
                            l.query.push_str(t);
                            l.selected = 0;
                        }
                    }
                }
            }
            refresh_launcher(app);
        }
        OverlayKind::Power => {
            let n = app.state.power.available.len();
            let p = &mut app.state.power;
            match key.keysym {
                ks::KEY_Left | ks::KEY_Up | ks::KEY_h | ks::KEY_k => {
                    p.selected = (p.selected + n - 1) % n.max(1);
                    p.confirm = None;
                }
                ks::KEY_Right | ks::KEY_Down | ks::KEY_l | ks::KEY_j | ks::KEY_Tab => {
                    p.selected = (p.selected + 1) % n.max(1);
                    p.confirm = None;
                }
                ks::KEY_Return | ks::KEY_KP_Enter | ks::KEY_space => {
                    let i = p.selected;
                    power_activate(app, platform, i);
                }
                _ => {
                    // First-letter shortcuts.
                    if let Some(c) = key.text.as_deref().and_then(|t| t.chars().next()) {
                        if let Some(i) = p.available.iter().position(|a| {
                            a.label()
                                .to_ascii_lowercase()
                                .starts_with(c.to_ascii_lowercase())
                        }) {
                            p.selected = i;
                            power_activate(app, platform, i);
                        }
                    }
                }
            }
        }
        OverlayKind::Notifications => {
            let v = &mut app.state.notifications;
            match key.keysym {
                ks::KEY_Up | ks::KEY_k => v.selected = v.selected.saturating_sub(1),
                ks::KEY_Down | ks::KEY_j => {
                    v.selected = (v.selected + 1).min(v.rows.len().saturating_sub(1))
                }
                ks::KEY_Delete | ks::KEY_BackSpace => {
                    if let Some(row) = v.rows.get(v.selected) {
                        let id = row.id;
                        app.store.history.retain(|h| h.id != id);
                        app.store.dismiss(id);
                        refresh_notifications(app);
                        crate::status::sync_toasts(app);
                        app.sync_toast_surface(platform);
                    }
                }
                ks::KEY_d => toggle_dnd(app, platform),
                ks::KEY_c => clear_notifications(app, platform),
                _ => {}
            }
        }
        OverlayKind::Settings => {
            crate::forms::key(app, platform, crate::forms::Which::Settings, key)
        }
        OverlayKind::Privacy => crate::forms::key(app, platform, crate::forms::Which::Privacy, key),
    }
}

fn toggle_dnd(app: &mut App, platform: &mut Platform<AppEvent>) {
    app.store.dnd = !app.store.dnd;
    app.config.notifications.dnd = app.store.dnd;
    app.commit_config(platform, false);
    refresh_notifications(app);
    crate::status::sync_toasts(app);
}

fn clear_notifications(app: &mut App, platform: &mut Platform<AppEvent>) {
    for id in app.store.dismiss_all() {
        if let Some(s) = &app.notif_server {
            s.emit_closed(id, notifications::server::CLOSE_DISMISSED);
        }
    }
    app.store.clear_history();
    refresh_notifications(app);
    crate::status::sync_toasts(app);
    app.sync_toast_surface(platform);
}

pub fn click(
    app: &mut App,
    platform: &mut Platform<AppEvent>,
    target: HitTarget,
    btn: u32,
    x: f64,
    y: f64,
) {
    let Some(kind) = app.state.overlay else {
        return;
    };
    if btn != button::LEFT {
        return;
    }
    match target {
        HitTarget::None => {
            app.close_overlay(platform);
            return;
        }
        HitTarget::OverlayClose => {
            app.close_overlay(platform);
            return;
        }
        HitTarget::OverlayPanel => return,
        _ => {}
    }
    match kind {
        OverlayKind::Launcher => {
            if let HitTarget::OverlayItem(i) = target {
                app.state.launcher.selected = i as usize;
                launch_selected(app, platform, false);
            }
        }
        OverlayKind::Power => {
            if let HitTarget::OverlayItem(i) = target {
                if app.state.power.selected != i as usize {
                    app.state.power.confirm = None;
                }
                app.state.power.selected = i as usize;
                power_activate(app, platform, i as usize);
            }
        }
        OverlayKind::Notifications => match target {
            HitTarget::OverlayItem(HIT_CLEAR_ALL) => clear_notifications(app, platform),
            HitTarget::OverlayItem(HIT_TOGGLE_DND) => toggle_dnd(app, platform),
            HitTarget::OverlayItem(id) => {
                if let Some(i) = app.state.notifications.rows.iter().position(|r| r.id == id) {
                    app.state.notifications.selected = i;
                }
            }
            _ => {}
        },
        OverlayKind::Settings => {
            crate::forms::click(app, platform, crate::forms::Which::Settings, target, x, y)
        }
        OverlayKind::Privacy => {
            crate::forms::click(app, platform, crate::forms::Which::Privacy, target, x, y)
        }
    }
}

pub fn scroll(app: &mut App, lines: i32) {
    match app.state.overlay {
        Some(OverlayKind::Launcher) => {
            let l = &mut app.state.launcher;
            l.scroll = (l.scroll as i32 + lines)
                .clamp(0, l.results.len().saturating_sub(VISIBLE_RESULTS) as i32)
                as usize;
            l.selected = l.selected.clamp(
                l.scroll,
                (l.scroll + VISIBLE_RESULTS).saturating_sub(1).max(l.scroll),
            );
        }
        Some(OverlayKind::Notifications) => {
            let v = &mut app.state.notifications;
            v.selected = (v.selected as i32 + lines).clamp(0, v.rows.len().saturating_sub(1) as i32)
                as usize;
        }
        Some(OverlayKind::Settings) => {
            let fs = &mut app.state.settings.form_state;
            fs.scroll = (fs.scroll + lines as f32 * app.state.metrics.line * 2.0).max(0.0);
        }
        Some(OverlayKind::Privacy) => {
            let fs = &mut app.state.privacy.form_state;
            fs.scroll = (fs.scroll + lines as f32 * app.state.metrics.line * 2.0).max(0.0);
        }
        _ => {}
    }
}

pub fn paste(app: &mut App, text: &str) {
    let text: String = text.chars().filter(|c| !c.is_control()).collect();
    match app.state.overlay {
        Some(OverlayKind::Launcher) => {
            app.state.launcher.query.push_str(&text);
            refresh_launcher(app);
        }
        Some(OverlayKind::Settings) => {
            crate::forms::insert_text(app, crate::forms::Which::Settings, &text)
        }
        Some(OverlayKind::Privacy) => {
            crate::forms::insert_text(app, crate::forms::Which::Privacy, &text)
        }
        _ => {}
    }
}
