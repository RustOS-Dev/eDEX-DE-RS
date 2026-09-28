//! Launch recency/frequency used to rank results.

use std::{collections::HashMap, path::PathBuf};

use serde::{Deserialize, Serialize};

#[derive(Debug, Default, Serialize, Deserialize)]
pub struct LaunchHistory {
    #[serde(default)]
    counts: HashMap<String, u32>,
    #[serde(default)]
    last: HashMap<String, u64>,
    #[serde(skip)]
    path: Option<PathBuf>,
}

impl LaunchHistory {
    pub fn load(path: PathBuf) -> Self {
        let mut h: LaunchHistory = std::fs::read_to_string(&path)
            .ok()
            .and_then(|s| serde_json::from_str(&s).ok())
            .unwrap_or_default();
        h.path = Some(path);
        h
    }

    pub fn record(&mut self, id: &str) {
        *self.counts.entry(id.to_string()).or_insert(0) += 1;
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_secs())
            .unwrap_or(0);
        self.last.insert(id.to_string(), now);
        if let Some(path) = &self.path {
            if let Some(dir) = path.parent() {
                let _ = std::fs::create_dir_all(dir);
            }
            if let Ok(json) = serde_json::to_string(self) {
                let _ = std::fs::write(path, json);
            }
        }
    }

    /// Score boost in 0..=1 for an app id.
    pub fn boost(&self, id: &str) -> f32 {
        let count = *self.counts.get(id).unwrap_or(&0) as f32;
        let recency = self.last.get(id).map(|t| {
            let now = std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map(|d| d.as_secs())
                .unwrap_or(0);
            let age_days = now.saturating_sub(*t) as f32 / 86_400.0;
            (1.0 - age_days / 30.0).clamp(0.0, 1.0)
        });
        (count.min(20.0) / 20.0) * 0.6 + recency.unwrap_or(0.0) * 0.4
    }

    pub fn recent(&self, n: usize) -> Vec<String> {
        let mut ids: Vec<(&String, &u64)> = self.last.iter().collect();
        ids.sort_by(|a, b| b.1.cmp(a.1));
        ids.into_iter().take(n).map(|(id, _)| id.clone()).collect()
    }
}
