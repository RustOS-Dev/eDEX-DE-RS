//! Generic form model used by the settings and privacy overlays.

#[derive(Clone, Debug, PartialEq)]
pub struct ListItem {
    pub label: String,
    pub detail: String,
    pub selected: bool,
    pub badge: Option<String>,
}

#[derive(Clone, Debug, PartialEq)]
pub enum ControlKind {
    Toggle(bool),
    Slider { value: f32, min: f32, max: f32, step: f32, unit: String },
    Choice { options: Vec<String>, selected: usize },
    Button(String),
    /// Editable text field (`secret` masks the value).
    Text { value: String, secret: bool, placeholder: String },
    Info(String),
    Progress { frac: f32, label: String },
    /// Selectable list with an optional per-item action button.
    List { items: Vec<ListItem>, action_label: Option<String>, empty: String },
    Separator,
    /// Multi-line read-only text.
    Note(String),
}

#[derive(Clone, Debug, PartialEq)]
pub struct Control {
    pub id: u32,
    pub label: String,
    pub kind: ControlKind,
    pub enabled: bool,
}

impl Control {
    pub fn new(id: u32, label: impl Into<String>, kind: ControlKind) -> Self {
        Self { id, label: label.into(), kind, enabled: true }
    }

    pub fn disabled(mut self) -> Self {
        self.enabled = false;
        self
    }

    pub fn is_focusable(&self) -> bool {
        self.enabled
            && !matches!(self.kind, ControlKind::Info(_) | ControlKind::Separator | ControlKind::Note(_) | ControlKind::Progress { .. })
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct Section {
    pub title: String,
    pub controls: Vec<Control>,
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct Form {
    pub sections: Vec<Section>,
}

impl Form {
    pub fn section(mut self, title: impl Into<String>, controls: Vec<Control>) -> Self {
        self.sections.push(Section { title: title.into(), controls });
        self
    }

    pub fn controls(&self) -> impl Iterator<Item = &Control> {
        self.sections.iter().flat_map(|s| s.controls.iter())
    }

    pub fn control(&self, id: u32) -> Option<&Control> {
        self.controls().find(|c| c.id == id)
    }

    pub fn control_mut(&mut self, id: u32) -> Option<&mut Control> {
        self.sections.iter_mut().flat_map(|s| s.controls.iter_mut()).find(|c| c.id == id)
    }

    pub fn focusable_ids(&self) -> Vec<u32> {
        self.controls().filter(|c| c.is_focusable()).map(|c| c.id).collect()
    }
}

/// Keyboard focus and scroll position inside a form.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct FormState {
    pub focused: Option<u32>,
    pub scroll: f32,
    /// Control currently being edited as text.
    pub editing: Option<u32>,
    /// Which list item is highlighted per list control.
    pub list_cursor: Vec<(u32, usize)>,
}

impl FormState {
    pub fn focus_next(&mut self, form: &Form, delta: i32) {
        let ids = form.focusable_ids();
        if ids.is_empty() {
            self.focused = None;
            return;
        }
        let pos = self.focused.and_then(|f| ids.iter().position(|i| *i == f)).map(|p| p as i32).unwrap_or(-1);
        let next = if pos < 0 && delta < 0 { ids.len() as i32 - 1 } else { (pos + delta).rem_euclid(ids.len() as i32) };
        self.focused = Some(ids[next as usize]);
    }

    pub fn list_cursor(&self, id: u32) -> usize {
        self.list_cursor.iter().find(|(i, _)| *i == id).map(|(_, c)| *c).unwrap_or(0)
    }

    pub fn set_list_cursor(&mut self, id: u32, cursor: usize) {
        if let Some(entry) = self.list_cursor.iter_mut().find(|(i, _)| *i == id) {
            entry.1 = cursor;
        } else {
            self.list_cursor.push((id, cursor));
        }
    }
}

/// Encode a hit id for a control part: `part` 0 = main, 1.. = option/item index + 1,
/// 250 = slider track, 251 = decrement, 252 = increment, 253 = item action button.
pub fn hit_id(control: u32, part: u8) -> u32 {
    (control << 8) | part as u32
}

pub fn decode_hit(id: u32) -> (u32, u8) {
    (id >> 8, (id & 0xff) as u8)
}

pub const PART_MAIN: u8 = 0;
pub const PART_SLIDER: u8 = 250;
pub const PART_DEC: u8 = 251;
pub const PART_INC: u8 = 252;
pub const PART_ACTION: u8 = 253;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn focus_cycles_over_focusable_controls() {
        let form = Form::default().section(
            "s",
            vec![
                Control::new(1, "a", ControlKind::Info("x".into())),
                Control::new(2, "b", ControlKind::Toggle(true)),
                Control::new(3, "c", ControlKind::Button("go".into())),
            ],
        );
        let mut st = FormState::default();
        st.focus_next(&form, 1);
        assert_eq!(st.focused, Some(2));
        st.focus_next(&form, 1);
        assert_eq!(st.focused, Some(3));
        st.focus_next(&form, 1);
        assert_eq!(st.focused, Some(2));
        st.focus_next(&form, -1);
        assert_eq!(st.focused, Some(3));
        assert_eq!(decode_hit(hit_id(42, PART_INC)), (42, PART_INC));
    }
}
