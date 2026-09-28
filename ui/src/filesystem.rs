//! Filesystem browser panel model.

use std::{
    fs,
    path::{Component, PathBuf},
    process::Command,
};

#[derive(Clone, Debug)]
pub struct FsEntry {
    pub name: String,
    pub path: PathBuf,
    pub is_dir: bool,
    pub size: Option<u64>,
    pub extension: Option<String>,
    pub is_hidden: bool,
    pub is_symlink: bool,
}

#[derive(Clone, Debug)]
pub struct FilesystemPanel {
    pub cwd: PathBuf,
    pub entries: Vec<FsEntry>,
    pub selected: usize,
    pub scroll_offset: usize,
    pub show_dotfiles: bool,
    pub max_visible: usize,
    pub error: Option<String>,
}

impl Default for FilesystemPanel {
    fn default() -> Self {
        Self::new()
    }
}

impl FilesystemPanel {
    pub fn new() -> Self {
        let cwd = std::env::var("HOME")
            .map(PathBuf::from)
            .unwrap_or_else(|_| PathBuf::from("/"));
        Self::at(cwd)
    }

    pub fn at(cwd: PathBuf) -> Self {
        let mut panel = Self {
            cwd,
            entries: Vec::new(),
            selected: 0,
            scroll_offset: 0,
            show_dotfiles: false,
            max_visible: 30,
            error: None,
        };
        panel.refresh();
        panel
    }

    pub fn refresh(&mut self) {
        let read = match fs::read_dir(&self.cwd) {
            Ok(r) => r,
            Err(e) => {
                self.error = Some(e.to_string());
                self.entries.clear();
                self.selected = 0;
                self.scroll_offset = 0;
                return;
            }
        };
        self.error = None;
        let mut entries: Vec<FsEntry> = read
            .flatten()
            .filter_map(|entry| {
                let path = entry.path();
                let name = entry.file_name().to_string_lossy().to_string();
                let is_hidden = name.starts_with('.');
                if is_hidden && !self.show_dotfiles {
                    return None;
                }
                let symlink_meta = fs::symlink_metadata(&path).ok()?;
                let metadata = fs::metadata(&path).ok();
                let is_dir = metadata.as_ref().is_some_and(|m| m.is_dir());
                let is_symlink = symlink_meta.file_type().is_symlink();
                let size = metadata
                    .as_ref()
                    .filter(|m| m.is_file())
                    .map(fs::Metadata::len);
                let extension = path.extension().map(|e| e.to_string_lossy().to_string());
                Some(FsEntry {
                    name,
                    path,
                    is_dir,
                    size,
                    extension,
                    is_hidden,
                    is_symlink,
                })
            })
            .collect();
        entries.sort_by(|a, b| {
            b.is_dir
                .cmp(&a.is_dir)
                .then_with(|| a.name.to_lowercase().cmp(&b.name.to_lowercase()))
        });
        self.entries = entries;
        self.selected = self.selected.min(self.entries.len().saturating_sub(1));
        self.clamp_scroll();
    }

    pub fn navigate_up(&mut self) {
        if self.selected > 0 {
            self.selected -= 1;
            self.clamp_scroll();
        }
    }

    pub fn navigate_down(&mut self) {
        if self.selected + 1 < self.entries.len() {
            self.selected += 1;
            self.clamp_scroll();
        }
    }

    pub fn page(&mut self, delta: i32) {
        let step = self.max_visible.max(1) as i32;
        let target = (self.selected as i32 + delta * step)
            .clamp(0, self.entries.len().saturating_sub(1) as i32);
        self.selected = target as usize;
        self.clamp_scroll();
    }

    pub fn scroll_by(&mut self, delta: i32) {
        let max_offset = self.entries.len().saturating_sub(self.max_visible);
        self.scroll_offset =
            (self.scroll_offset as i32 + delta).clamp(0, max_offset as i32) as usize;
    }

    /// Select the entry at a visible index (from a click).
    pub fn select_visible(&mut self, visible_index: usize) {
        let idx = self.scroll_offset + visible_index;
        if idx < self.entries.len() {
            self.selected = idx;
        }
    }

    /// Enter the selected directory or open the selected file with the default handler.
    pub fn enter_selected(&mut self) {
        let Some(entry) = self.entries.get(self.selected).cloned() else {
            return;
        };
        if entry.is_dir {
            self.cwd = entry.path;
            self.selected = 0;
            self.scroll_offset = 0;
            self.refresh();
        } else {
            open_external(&entry.path);
        }
    }

    pub fn go_parent(&mut self) {
        if let Some(parent) = self.cwd.parent().map(PathBuf::from) {
            let previous = self
                .cwd
                .file_name()
                .map(|n| n.to_string_lossy().to_string());
            self.cwd = parent;
            self.selected = 0;
            self.scroll_offset = 0;
            self.refresh();
            if let Some(prev) = previous {
                if let Some(i) = self.entries.iter().position(|e| e.name == prev) {
                    self.selected = i;
                    self.clamp_scroll();
                }
            }
        }
    }

    /// Jump to the n-th breadcrumb (0 = root).
    pub fn go_breadcrumb(&mut self, index: usize) {
        let crumbs = self.breadcrumb_paths();
        if let Some(path) = crumbs.get(index) {
            self.cwd = path.clone();
            self.selected = 0;
            self.scroll_offset = 0;
            self.refresh();
        }
    }

    pub fn toggle_dotfiles(&mut self) {
        self.show_dotfiles = !self.show_dotfiles;
        self.selected = 0;
        self.scroll_offset = 0;
        self.refresh();
    }

    pub fn set_max_visible(&mut self, rows: usize) {
        self.max_visible = rows.max(1);
        self.clamp_scroll();
    }

    pub fn visible_entries(&self) -> &[FsEntry] {
        let end = (self.scroll_offset + self.max_visible).min(self.entries.len());
        &self.entries[self.scroll_offset.min(end)..end]
    }

    pub fn breadcrumbs(&self) -> Vec<String> {
        let mut crumbs = Vec::new();
        for component in self.cwd.components() {
            match component {
                Component::RootDir => crumbs.push("/".to_string()),
                Component::Normal(part) => crumbs.push(part.to_string_lossy().to_string()),
                Component::Prefix(prefix) => {
                    crumbs.push(prefix.as_os_str().to_string_lossy().to_string())
                }
                _ => {}
            }
        }
        if crumbs.is_empty() {
            crumbs.push("/".to_string());
        }
        crumbs
    }

    fn breadcrumb_paths(&self) -> Vec<PathBuf> {
        let mut paths = Vec::new();
        let mut acc = PathBuf::new();
        for component in self.cwd.components() {
            match component {
                Component::RootDir => {
                    acc.push("/");
                    paths.push(acc.clone());
                }
                Component::Normal(part) => {
                    acc.push(part);
                    paths.push(acc.clone());
                }
                Component::Prefix(prefix) => {
                    acc.push(prefix.as_os_str());
                    paths.push(acc.clone());
                }
                _ => {}
            }
        }
        paths
    }

    pub fn selected_visible_index(&self) -> Option<usize> {
        self.selected
            .checked_sub(self.scroll_offset)
            .filter(|i| *i < self.max_visible)
    }

    fn clamp_scroll(&mut self) {
        if self.selected < self.scroll_offset {
            self.scroll_offset = self.selected;
        }
        if self.selected >= self.scroll_offset + self.max_visible {
            self.scroll_offset = self.selected + 1 - self.max_visible;
        }
    }
}

/// Launch `xdg-open` detached from the shell so it never blocks and never leaves a zombie.
pub fn open_external(path: &std::path::Path) {
    let path = path.to_path_buf();
    std::thread::spawn(move || {
        if let Ok(mut child) = Command::new("xdg-open").arg(&path).spawn() {
            let _ = child.wait();
        }
    });
}

pub fn format_size(bytes: u64) -> String {
    const K: f64 = 1024.0;
    let b = bytes as f64;
    if b >= K * K * K {
        format!("{:.1}G", b / (K * K * K))
    } else if b >= K * K {
        format!("{:.1}M", b / (K * K))
    } else if b >= K {
        format!("{:.0}K", b / K)
    } else {
        format!("{bytes}B")
    }
}

pub fn icon_for_entry(entry: &FsEntry) -> &'static str {
    if entry.is_dir {
        "▸"
    } else if entry.is_symlink {
        "↪"
    } else {
        match entry
            .extension
            .as_deref()
            .map(str::to_ascii_lowercase)
            .as_deref()
        {
            Some("rs") | Some("c") | Some("h") | Some("py") | Some("js") | Some("ts")
            | Some("go") | Some("lua") => "λ",
            Some("toml") | Some("yaml") | Some("yml") | Some("json") | Some("conf")
            | Some("ini") => "⚙",
            Some("md") | Some("txt") | Some("pdf") | Some("doc") | Some("odt") => "≡",
            Some("png") | Some("jpg") | Some("jpeg") | Some("gif") | Some("svg") | Some("webp") => {
                "▣"
            }
            Some("zip") | Some("tar") | Some("gz") | Some("xz") | Some("zst") | Some("7z") => "▤",
            Some("sh") | Some("fish") | Some("bash") => "$",
            Some("mp3") | Some("flac") | Some("ogg") | Some("wav") => "♫",
            Some("mp4") | Some("mkv") | Some("webm") => "▶",
            _ => "·",
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn navigates_and_scrolls() {
        let dir = std::env::temp_dir().join(format!("edex-fs-test-{}", std::process::id()));
        fs::create_dir_all(dir.join("sub")).unwrap();
        for i in 0..10 {
            fs::write(dir.join(format!("f{i:02}.txt")), "x").unwrap();
        }
        fs::write(dir.join(".hidden"), "x").unwrap();
        let mut panel = FilesystemPanel::at(dir.clone());
        assert_eq!(panel.entries.len(), 11);
        assert!(panel.entries[0].is_dir);
        panel.set_max_visible(4);
        for _ in 0..6 {
            panel.navigate_down();
        }
        assert_eq!(panel.selected, 6);
        assert_eq!(panel.scroll_offset, 3);
        assert_eq!(panel.selected_visible_index(), Some(3));
        panel.toggle_dotfiles();
        assert_eq!(panel.entries.len(), 12);
        panel.selected = 0;
        panel.enter_selected();
        assert!(panel.cwd.ends_with("sub"));
        panel.go_parent();
        assert_eq!(panel.cwd, dir);
        assert_eq!(panel.entries[panel.selected].name, "sub");
        assert_eq!(format_size(2048), "2K");
        fs::remove_dir_all(dir).unwrap();
    }
}
