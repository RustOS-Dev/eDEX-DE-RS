//! Fuzzy search over name, generic name, keywords and comment with recency boost.

use fuzzy_matcher::{skim::SkimMatcherV2, FuzzyMatcher};

use crate::{desktop::AppEntry, history::LaunchHistory};

pub struct AppSearch {
    matcher: SkimMatcherV2,
}

impl Default for AppSearch {
    fn default() -> Self {
        Self::new()
    }
}

impl AppSearch {
    pub fn new() -> Self {
        Self { matcher: SkimMatcherV2::default().ignore_case() }
    }

    /// Rank apps for a query; an empty query lists recently used apps first.
    pub fn search<'a>(&self, query: &str, apps: &'a [AppEntry], history: &LaunchHistory, limit: usize) -> Vec<&'a AppEntry> {
        let query = query.trim();
        if query.is_empty() {
            let recent = history.recent(limit);
            let mut out: Vec<&AppEntry> = recent.iter().filter_map(|id| apps.iter().find(|a| &a.id == id)).collect();
            for app in apps {
                if out.len() >= limit {
                    break;
                }
                if !out.iter().any(|a| a.id == app.id) {
                    out.push(app);
                }
            }
            return out;
        }
        let mut scored: Vec<(f64, &AppEntry)> = apps
            .iter()
            .filter_map(|app| {
                let name = self.matcher.fuzzy_match(&app.name, query).map(|s| s as f64 * 1.0);
                let generic = app.generic_name.as_deref().and_then(|g| self.matcher.fuzzy_match(g, query)).map(|s| s as f64 * 0.8);
                let keywords = app.keywords.iter().filter_map(|k| self.matcher.fuzzy_match(k, query)).max().map(|s| s as f64 * 0.7);
                let comment = app.comment.as_deref().and_then(|c| self.matcher.fuzzy_match(c, query)).map(|s| s as f64 * 0.4);
                let exec = self.matcher.fuzzy_match(&app.exec, query).map(|s| s as f64 * 0.3);
                let best = [name, generic, keywords, comment, exec].into_iter().flatten().fold(None, |acc: Option<f64>, s| Some(acc.map_or(s, |a| a.max(s))))?;
                let prefix_bonus = if app.name.to_lowercase().starts_with(&query.to_lowercase()) { 40.0 } else { 0.0 };
                Some((best + prefix_bonus + history.boost(&app.id) as f64 * 30.0, app))
            })
            .collect();
        scored.sort_by(|a, b| b.0.partial_cmp(&a.0).unwrap_or(std::cmp::Ordering::Equal).then_with(|| a.1.name.cmp(&b.1.name)));
        scored.into_iter().take(limit).map(|(_, a)| a).collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    fn app(id: &str, name: &str, kw: &[&str]) -> AppEntry {
        AppEntry {
            id: id.into(),
            name: name.into(),
            generic_name: None,
            exec: name.to_lowercase(),
            icon: None,
            categories: vec![],
            keywords: kw.iter().map(|s| s.to_string()).collect(),
            comment: None,
            terminal: false,
            path: None,
            startup_wm_class: None,
            file: PathBuf::from("/dev/null"),
        }
    }

    #[test]
    fn ranks_prefix_and_keywords() {
        let apps = vec![app("ff", "Firefox", &["browser", "web"]), app("files", "Files", &[]), app("kitty", "kitty", &["terminal"])];
        let search = AppSearch::new();
        let history = LaunchHistory::default();
        let r = search.search("fire", &apps, &history, 8);
        assert_eq!(r[0].name, "Firefox");
        let r = search.search("browser", &apps, &history, 8);
        assert_eq!(r[0].name, "Firefox");
        let r = search.search("term", &apps, &history, 8);
        assert_eq!(r[0].name, "kitty");
        assert!(search.search("zzzz", &apps, &history, 8).is_empty());
    }
}
