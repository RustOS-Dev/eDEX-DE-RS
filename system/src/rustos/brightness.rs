//! Backlight brightness on RustOS: `/sys/class/backlight/*/{brightness,max_brightness}`.

use std::path::{Path, PathBuf};

use anyhow::{anyhow, Context, Result};

use crate::brightness::BrightnessState;

fn first_backlight(root: &Path) -> Option<PathBuf> {
    let mut v: Vec<PathBuf> = std::fs::read_dir(root)
        .ok()?
        .flatten()
        .map(|e| e.path())
        .collect();
    v.sort();
    v.into_iter().next()
}

fn value(dir: &Path, name: &str) -> Option<u64> {
    std::fs::read_to_string(dir.join(name))
        .ok()?
        .trim()
        .parse()
        .ok()
}

pub fn query_at(root: &Path) -> BrightnessState {
    let Some(dir) = first_backlight(root) else {
        return BrightnessState::default();
    };
    match (value(&dir, "brightness"), value(&dir, "max_brightness")) {
        (Some(b), Some(m)) if m > 0 => BrightnessState {
            percent: ((b * 100 + m / 2) / m) as u32,
            available: true,
            device: dir
                .file_name()
                .map(|n| n.to_string_lossy().into_owned())
                .unwrap_or_default(),
        },
        _ => BrightnessState::default(),
    }
}

pub fn set_at(root: &Path, percent: u32) -> Result<BrightnessState> {
    let dir = first_backlight(root).ok_or_else(|| anyhow!("no backlight"))?;
    let max = value(&dir, "max_brightness").ok_or_else(|| anyhow!("no max_brightness"))?;
    let raw = (max * percent.clamp(1, 100) as u64 + 50) / 100;
    std::fs::write(dir.join("brightness"), format!("{raw}\n"))
        .with_context(|| format!("writing {}", dir.join("brightness").display()))?;
    Ok(query_at(root))
}

const ROOT: &str = "/sys/class/backlight";

pub fn query() -> BrightnessState {
    query_at(Path::new(ROOT))
}

pub fn set(percent: u32) -> Result<BrightnessState> {
    set_at(Path::new(ROOT), percent)
}

pub fn adjust(delta: i32) -> Result<BrightnessState> {
    let now = query();
    if !now.available {
        return Err(anyhow!("no backlight"));
    }
    set((now.percent as i32 + delta).clamp(1, 100) as u32)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reads_and_writes_sysfs() {
        let t = tempfile::tempdir().unwrap();
        let d = t.path().join("amdgpu_bl0");
        std::fs::create_dir_all(&d).unwrap();
        std::fs::write(d.join("max_brightness"), "255\n").unwrap();
        std::fs::write(d.join("brightness"), "128\n").unwrap();
        let st = query_at(t.path());
        assert!(st.available);
        assert_eq!(st.percent, 50);
        assert_eq!(st.device, "amdgpu_bl0");
        let st = set_at(t.path(), 20).unwrap();
        assert_eq!(
            std::fs::read_to_string(d.join("brightness")).unwrap(),
            "51\n"
        );
        assert_eq!(st.percent, 20);
        assert!(!query_at(&t.path().join("none")).available);
    }
}
