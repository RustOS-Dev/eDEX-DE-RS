//! Shared interaction logic for the tabbed forms (settings and privacy overlays).

pub mod privacy;
pub mod settings;

use platform::{KeyInput, Platform};
use ui::{
    form::{decode_hit, ControlKind, Form, PART_ACTION, PART_MAIN, PART_SLIDER},
    hit::HitTarget,
    overlays::form_view::{HIT_CLOSE, HIT_TAB_BASE},
    state::TabbedForms,
};
use xkbcommon::xkb::keysyms as ks;

use crate::{app::App, events::AppEvent};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Which {
    Settings,
    Privacy,
}

/// A user edit to one control.
#[derive(Clone, Debug, PartialEq)]
pub enum Change {
    Toggle(bool),
    Slider(f32),
    Choice(usize),
    Button,
    Text(String),
    ListSelect(usize),
    ListAction(usize),
}

pub fn forms_mut(app: &mut App, which: Which) -> &mut TabbedForms {
    match which {
        Which::Settings => &mut app.state.settings,
        Which::Privacy => &mut app.state.privacy,
    }
}

pub fn forms(app: &App, which: Which) -> &TabbedForms {
    match which {
        Which::Settings => &app.state.settings,
        Which::Privacy => &app.state.privacy,
    }
}

fn rebuild(app: &mut App, which: Which) {
    match which {
        Which::Settings => settings::rebuild(app),
        Which::Privacy => privacy::rebuild(app),
    }
}

fn select_tab(app: &mut App, which: Which, tab: usize) {
    match which {
        Which::Settings => settings::select_tab(app, tab),
        Which::Privacy => privacy::select_tab(app, tab),
    }
}

fn dispatch(
    app: &mut App,
    platform: &mut Platform<AppEvent>,
    which: Which,
    id: u32,
    change: Change,
) {
    match which {
        Which::Settings => settings::on_change(app, platform, id, change),
        Which::Privacy => privacy::on_change(app, platform, id, change),
    }
    if app.state.overlay.is_some() {
        rebuild(app, which);
    }
}

/// Stop editing the active text control and apply what was typed (leaving a field saves it, the
/// same as pressing Enter).
fn commit_editing(app: &mut App, platform: &mut Platform<AppEvent>, which: Which) {
    let f = forms_mut(app, which);
    let Some(id) = f.form_state.editing.take() else {
        return;
    };
    let value = match f.form.control(id).map(|c| &c.kind) {
        Some(ControlKind::Text { value, .. }) => value.clone(),
        _ => return,
    };
    dispatch(app, platform, which, id, Change::Text(value));
}

/// Text typed into the focused text control.
pub fn insert_text(app: &mut App, which: Which, text: &str) {
    let f = forms_mut(app, which);
    let Some(id) = f.form_state.editing else {
        return;
    };
    if let Some(c) = f.form.control_mut(id) {
        if let ControlKind::Text { value, .. } = &mut c.kind {
            value.push_str(text);
        }
    }
}

fn adjust(form: &Form, id: u32, dir: i32) -> Option<Change> {
    let c = form.control(id)?;
    match &c.kind {
        ControlKind::Slider {
            value,
            min,
            max,
            step,
            ..
        } => {
            let v = (value + dir as f32 * step).clamp(*min, *max);
            Some(Change::Slider(v))
        }
        ControlKind::Choice { options, selected } => {
            if options.is_empty() {
                return None;
            }
            let n = options.len() as i32;
            Some(Change::Choice(
                ((*selected as i32 + dir).rem_euclid(n)) as usize,
            ))
        }
        _ => None,
    }
}

pub fn key(app: &mut App, platform: &mut Platform<AppEvent>, which: Which, key: &KeyInput) {
    // Editing a text field captures everything but Escape (handled by the caller).
    let editing = forms(app, which).form_state.editing;
    if let Some(id) = editing {
        match key.keysym {
            ks::KEY_Return | ks::KEY_KP_Enter => {
                let f = forms_mut(app, which);
                let value = match f.form.control(id).map(|c| &c.kind) {
                    Some(ControlKind::Text { value, .. }) => value.clone(),
                    _ => String::new(),
                };
                f.form_state.editing = None;
                dispatch(app, platform, which, id, Change::Text(value));
            }
            ks::KEY_BackSpace => {
                let f = forms_mut(app, which);
                if let Some(c) = f.form.control_mut(id) {
                    if let ControlKind::Text { value, .. } = &mut c.kind {
                        if key.modifiers.ctrl {
                            value.clear();
                        } else {
                            value.pop();
                        }
                    }
                }
            }
            ks::KEY_Tab => {
                commit_editing(app, platform, which);
                let f = forms_mut(app, which);
                let form = f.form.clone();
                f.form_state
                    .focus_next(&form, if key.modifiers.shift { -1 } else { 1 });
            }
            _ => {
                if let Some(t) = key.text.as_deref() {
                    if !key.modifiers.ctrl
                        && !key.modifiers.alt
                        && !t.chars().any(|c| c.is_control())
                    {
                        insert_text(app, which, t);
                    }
                }
            }
        }
        return;
    }
    let tabs = forms(app, which).tabs.len();
    let active = forms(app, which).active;
    match key.keysym {
        ks::KEY_Page_Up if key.modifiers.ctrl => {
            select_tab(app, which, (active + tabs - 1) % tabs.max(1));
            return;
        }
        ks::KEY_Page_Down if key.modifiers.ctrl => {
            select_tab(app, which, (active + 1) % tabs.max(1));
            return;
        }
        ks::KEY_Page_Up => {
            let line = app.state.metrics.line;
            let f = forms_mut(app, which);
            f.form_state.scroll = (f.form_state.scroll - line * 10.0).max(0.0);
            return;
        }
        ks::KEY_Page_Down => {
            let line = app.state.metrics.line;
            let f = forms_mut(app, which);
            f.form_state.scroll += line * 10.0;
            return;
        }
        ks::KEY_Tab | ks::KEY_Down | ks::KEY_j if !key.modifiers.shift => {
            let f = forms_mut(app, which);
            let form = f.form.clone();
            f.form_state.focus_next(&form, 1);
            return;
        }
        ks::KEY_ISO_Left_Tab | ks::KEY_Up | ks::KEY_k => {
            let f = forms_mut(app, which);
            let form = f.form.clone();
            f.form_state.focus_next(&form, -1);
            return;
        }
        ks::KEY_Tab => {
            let f = forms_mut(app, which);
            let form = f.form.clone();
            f.form_state.focus_next(&form, -1);
            return;
        }
        _ => {}
    }
    if (ks::KEY_1..=ks::KEY_9).contains(&key.keysym) && key.modifiers.alt {
        let i = (key.keysym - ks::KEY_1) as usize;
        if i < tabs {
            select_tab(app, which, i);
        }
        return;
    }
    let Some(id) = forms(app, which).form_state.focused else {
        return;
    };
    let kind = forms(app, which)
        .form
        .control(id)
        .map(|c| (c.kind.clone(), c.enabled));
    let Some((kind, enabled)) = kind else { return };
    if !enabled {
        return;
    }
    match key.keysym {
        ks::KEY_Left | ks::KEY_h | ks::KEY_minus => match &kind {
            ControlKind::List { items, .. } => {
                let f = forms_mut(app, which);
                let cur = f.form_state.list_cursor(id);
                if !items.is_empty() {
                    f.form_state.set_list_cursor(id, cur.saturating_sub(1));
                    dispatch(
                        app,
                        platform,
                        which,
                        id,
                        Change::ListSelect(cur.saturating_sub(1)),
                    );
                }
            }
            _ => {
                if let Some(ch) = adjust(&forms(app, which).form, id, -1) {
                    dispatch(app, platform, which, id, ch);
                }
            }
        },
        ks::KEY_Right | ks::KEY_l | ks::KEY_plus | ks::KEY_equal => match &kind {
            ControlKind::List { items, .. } => {
                let f = forms_mut(app, which);
                let cur = f.form_state.list_cursor(id);
                if cur + 1 < items.len() {
                    f.form_state.set_list_cursor(id, cur + 1);
                    dispatch(app, platform, which, id, Change::ListSelect(cur + 1));
                }
            }
            _ => {
                if let Some(ch) = adjust(&forms(app, which).form, id, 1) {
                    dispatch(app, platform, which, id, ch);
                }
            }
        },
        ks::KEY_Return | ks::KEY_KP_Enter | ks::KEY_space => match kind {
            ControlKind::Toggle(v) => dispatch(app, platform, which, id, Change::Toggle(!v)),
            ControlKind::Button(_) => dispatch(app, platform, which, id, Change::Button),
            ControlKind::Text { .. } => forms_mut(app, which).form_state.editing = Some(id),
            ControlKind::Choice { .. } => {
                if let Some(ch) = adjust(&forms(app, which).form, id, 1) {
                    dispatch(app, platform, which, id, ch);
                }
            }
            ControlKind::List { .. } => {
                let cur = forms(app, which).form_state.list_cursor(id);
                dispatch(app, platform, which, id, Change::ListAction(cur));
            }
            _ => {}
        },
        _ => {}
    }
}

pub fn click(
    app: &mut App,
    platform: &mut Platform<AppEvent>,
    which: Which,
    target: HitTarget,
    x: f64,
    _y: f64,
) {
    let HitTarget::OverlayItem(raw) = target else {
        return;
    };
    if raw == HIT_CLOSE {
        app.close_overlay(platform);
        return;
    }
    if (HIT_TAB_BASE..HIT_CLOSE).contains(&raw) {
        select_tab(app, which, (raw - HIT_TAB_BASE) as usize);
        return;
    }
    let item_index = (raw >> 24) as usize;
    let (id, part) = decode_hit(raw & 0x00ff_ffff);
    let kind = forms(app, which)
        .form
        .control(id)
        .map(|c| (c.kind.clone(), c.enabled));
    let Some((kind, enabled)) = kind else { return };
    if forms(app, which).form_state.editing != Some(id) {
        commit_editing(app, platform, which);
    }
    forms_mut(app, which).form_state.focused = Some(id);
    if !enabled {
        return;
    }
    match (part, kind) {
        (PART_MAIN, ControlKind::Toggle(v)) => {
            dispatch(app, platform, which, id, Change::Toggle(!v))
        }
        (PART_MAIN, ControlKind::Button(_)) => dispatch(app, platform, which, id, Change::Button),
        (PART_MAIN, ControlKind::Text { .. }) => {
            forms_mut(app, which).form_state.editing = Some(id)
        }
        (PART_SLIDER, ControlKind::Slider { min, max, step, .. }) => {
            let rect = app.hits.values().find_map(|h| h.rect_of(target));
            let Some(rect) = rect else { return };
            let frac = ((x as f32 - rect.x) / rect.w.max(1.0)).clamp(0.0, 1.0);
            let raw_v = min + frac * (max - min);
            let v = if step > 0.0 {
                ((raw_v / step).round() * step).clamp(min, max)
            } else {
                raw_v
            };
            dispatch(app, platform, which, id, Change::Slider(v));
        }
        (p, ControlKind::Choice { options, .. }) if p >= 1 && (p as usize) <= options.len() => {
            dispatch(app, platform, which, id, Change::Choice(p as usize - 1));
        }
        (PART_ACTION, ControlKind::List { .. }) => {
            forms_mut(app, which)
                .form_state
                .set_list_cursor(id, item_index);
            dispatch(app, platform, which, id, Change::ListAction(item_index));
        }
        (p, ControlKind::List { items, .. }) if p >= 1 && (p as usize) <= items.len() => {
            let i = p as usize - 1;
            forms_mut(app, which).form_state.set_list_cursor(id, i);
            dispatch(app, platform, which, id, Change::ListSelect(i));
        }
        _ => {}
    }
}

/// Helpers for building controls.
pub mod build {
    use ui::form::{Control, ControlKind, ListItem};

    pub fn toggle(id: u32, label: &str, v: bool) -> Control {
        Control::new(id, label, ControlKind::Toggle(v))
    }
    pub fn slider(
        id: u32,
        label: &str,
        value: f32,
        min: f32,
        max: f32,
        step: f32,
        unit: &str,
    ) -> Control {
        Control::new(
            id,
            label,
            ControlKind::Slider {
                value,
                min,
                max,
                step,
                unit: unit.into(),
            },
        )
    }
    pub fn choice(id: u32, label: &str, options: &[&str], selected: usize) -> Control {
        Control::new(
            id,
            label,
            ControlKind::Choice {
                options: options.iter().map(|s| s.to_string()).collect(),
                selected,
            },
        )
    }
    pub fn choice_owned(id: u32, label: &str, options: Vec<String>, selected: usize) -> Control {
        Control::new(id, label, ControlKind::Choice { options, selected })
    }
    pub fn button(id: u32, label: &str, text: &str) -> Control {
        Control::new(id, label, ControlKind::Button(text.into()))
    }
    pub fn text(id: u32, label: &str, value: &str, placeholder: &str) -> Control {
        Control::new(
            id,
            label,
            ControlKind::Text {
                value: value.into(),
                secret: false,
                placeholder: placeholder.into(),
            },
        )
    }
    pub fn secret(id: u32, label: &str, value: &str, placeholder: &str) -> Control {
        Control::new(
            id,
            label,
            ControlKind::Text {
                value: value.into(),
                secret: true,
                placeholder: placeholder.into(),
            },
        )
    }
    pub fn info(id: u32, label: &str, value: impl Into<String>) -> Control {
        Control::new(id, label, ControlKind::Info(value.into()))
    }
    pub fn progress(id: u32, label: &str, frac: f32, text: &str) -> Control {
        Control::new(
            id,
            label,
            ControlKind::Progress {
                frac,
                label: text.into(),
            },
        )
    }
    pub fn list(
        id: u32,
        label: &str,
        items: Vec<ListItem>,
        action: Option<&str>,
        empty: &str,
    ) -> Control {
        Control::new(
            id,
            label,
            ControlKind::List {
                items,
                action_label: action.map(|s| s.to_string()),
                empty: empty.into(),
            },
        )
    }
    pub fn note(id: u32, text: &str) -> Control {
        Control::new(id, "", ControlKind::Note(text.into()))
    }
    pub fn item(
        label: impl Into<String>,
        detail: impl Into<String>,
        selected: bool,
        badge: Option<&str>,
    ) -> ListItem {
        ListItem {
            label: label.into(),
            detail: detail.into(),
            selected,
            badge: badge.map(|s| s.to_string()),
        }
    }
    pub fn secs(v: u32) -> String {
        if v == 0 {
            "never".into()
        } else if v.is_multiple_of(60) {
            format!("{} min", v / 60)
        } else {
            format!("{v} s")
        }
    }
}
