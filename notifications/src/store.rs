//! Active toasts, history and policy (DND, per-app mute, timeouts).

use std::{
    collections::VecDeque,
    path::Path,
    time::{Duration, Instant, SystemTime, UNIX_EPOCH},
};

use serde::{Deserialize, Serialize};

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize, Default)]
pub enum Urgency {
    Low = 0,
    #[default]
    Normal = 1,
    Critical = 2,
}

impl From<u8> for Urgency {
    fn from(v: u8) -> Self {
        match v {
            0 => Urgency::Low,
            2 => Urgency::Critical,
            _ => Urgency::Normal,
        }
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct Notification {
    pub id: u32,
    pub app: String,
    pub icon: String,
    pub summary: String,
    pub body: String,
    /// (action key, label) pairs.
    pub actions: Vec<(String, String)>,
    pub urgency: Urgency,
    /// `None` = server default; `Some(0)` = never expires.
    pub timeout_ms: Option<u32>,
    /// Progress from the `value` hint.
    pub progress: Option<f32>,
    pub transient: bool,
    pub desktop_entry: Option<String>,
    pub created: Instant,
    pub wall_time: u64,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub struct HistoryEntry {
    pub id: u32,
    pub app: String,
    pub summary: String,
    pub body: String,
    pub urgency: Urgency,
    pub time: u64,
}

pub struct NotificationStore {
    pub active: Vec<Notification>,
    pub history: VecDeque<HistoryEntry>,
    pub dnd: bool,
    pub muted_apps: Vec<String>,
    pub default_timeout: Duration,
    pub max_visible: usize,
    pub history_limit: usize,
    pub unread: usize,
}

impl Default for NotificationStore {
    fn default() -> Self {
        Self { active: Vec::new(), history: VecDeque::new(), dnd: false, muted_apps: Vec::new(), default_timeout: Duration::from_millis(5000), max_visible: 4, history_limit: 200, unread: 0 }
    }
}

impl NotificationStore {
    /// Record a notification. Returns whether a toast should be shown.
    pub fn push(&mut self, mut n: Notification) -> bool {
        // Replace an existing notification with the same id.
        self.active.retain(|a| a.id != n.id);
        n.wall_time = SystemTime::now().duration_since(UNIX_EPOCH).map(|d| d.as_secs()).unwrap_or(0);
        let muted = self.muted_apps.iter().any(|m| m.eq_ignore_ascii_case(&n.app));
        let show = !muted && (!self.dnd || n.urgency == Urgency::Critical);
        if !n.transient {
            self.history.push_front(HistoryEntry { id: n.id, app: n.app.clone(), summary: n.summary.clone(), body: n.body.clone(), urgency: n.urgency, time: n.wall_time });
            while self.history.len() > self.history_limit {
                self.history.pop_back();
            }
            self.unread += 1;
        }
        if show {
            self.active.push(n);
            while self.active.len() > self.max_visible {
                // Drop the oldest non-critical toast first.
                if let Some(pos) = self.active.iter().position(|a| a.urgency != Urgency::Critical) {
                    self.active.remove(pos);
                } else {
                    self.active.remove(0);
                }
            }
        }
        show
    }

    /// Remove expired toasts; returns their ids.
    pub fn expire(&mut self, now: Instant) -> Vec<u32> {
        let default = self.default_timeout;
        let mut expired = Vec::new();
        self.active.retain(|n| {
            let timeout = match (n.timeout_ms, n.urgency) {
                (_, Urgency::Critical) => None,
                (Some(0), _) => None,
                (Some(ms), _) => Some(Duration::from_millis(ms as u64)),
                (None, _) => Some(default),
            };
            match timeout {
                Some(t) if now.duration_since(n.created) >= t => {
                    expired.push(n.id);
                    false
                }
                _ => true,
            }
        });
        expired
    }

    pub fn dismiss(&mut self, id: u32) -> bool {
        let before = self.active.len();
        self.active.retain(|n| n.id != id);
        before != self.active.len()
    }

    pub fn dismiss_all(&mut self) -> Vec<u32> {
        self.active.drain(..).map(|n| n.id).collect()
    }

    pub fn clear_history(&mut self) {
        self.history.clear();
        self.unread = 0;
    }

    pub fn mark_read(&mut self) {
        self.unread = 0;
    }

    pub fn active_action(&self, id: u32, index: usize) -> Option<String> {
        self.active.iter().find(|n| n.id == id).and_then(|n| n.actions.get(index)).map(|(k, _)| k.clone())
    }

    /// Persist the history to disk (JSON).
    pub fn save_history(&self, path: &Path) {
        if let Some(dir) = path.parent() {
            let _ = std::fs::create_dir_all(dir);
        }
        if let Ok(json) = serde_json::to_string(&self.history.iter().collect::<Vec<_>>()) {
            let _ = std::fs::write(path, json);
        }
    }

    pub fn load_history(&mut self, path: &Path) {
        if let Ok(text) = std::fs::read_to_string(path) {
            if let Ok(entries) = serde_json::from_str::<Vec<HistoryEntry>>(&text) {
                self.history = entries.into_iter().take(self.history_limit).collect();
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn n(id: u32, app: &str, urgency: Urgency, timeout: Option<u32>) -> Notification {
        Notification { id, app: app.into(), icon: String::new(), summary: "s".into(), body: "b".into(), actions: vec![], urgency, timeout_ms: timeout, progress: None, transient: false, desktop_entry: None, created: Instant::now(), wall_time: 0 }
    }

    #[test]
    fn dnd_mute_and_expiry_policy() {
        let mut s = NotificationStore { default_timeout: Duration::from_millis(10), ..Default::default() };
        assert!(s.push(n(1, "a", Urgency::Normal, None)));
        s.dnd = true;
        assert!(!s.push(n(2, "a", Urgency::Normal, None)));
        assert!(s.push(n(3, "a", Urgency::Critical, None)));
        s.dnd = false;
        s.muted_apps.push("spam".into());
        assert!(!s.push(n(4, "Spam", Urgency::Normal, None)));
        assert_eq!(s.history.len(), 4);
        assert_eq!(s.unread, 4);
        std::thread::sleep(Duration::from_millis(20));
        let expired = s.expire(Instant::now());
        assert_eq!(expired, vec![1]);
        assert_eq!(s.active.len(), 1);
        assert!(s.dismiss(3));
        assert!(s.active.is_empty());
    }
}
