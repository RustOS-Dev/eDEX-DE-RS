//! Backlight control through `/sys/class/backlight` (RustOS exposes the DRM drivers' panels
//! there; no brightnessctl).

use std::{
    fs,
    path::{Path, PathBuf},
};

use anyhow::{anyhow, Context, Result};

#[derive(Clone, Debug, Default, PartialEq)]
pub struct BrightnessState {
    pub percent: u32,
    pub available: bool,
    pub device: String,
}

pub const SYSFS_ROOT: &str = "/sys/class/backlight";

/// The backlight to drive: firmware (ACPI) interfaces first, then platform, then raw GPU
/// registers, the order systemd and brightnessctl use.
fn pick(root: &Path) -> Option<PathBuf> {
    let mut devices: Vec<(u8, PathBuf)> = fs::read_dir(root)
        .ok()?
        .flatten()
        .map(|e| e.path())
        .filter(|p| p.join("max_brightness").exists())
        .map(|p| {
            let rank = match read(&p.join("type")).as_deref() {
                Some("firmware") => 0,
                Some("platform") => 1,
                _ => 2,
            };
            (rank, p)
        })
        .collect();
    devices.sort();
    devices.into_iter().next().map(|(_, p)| p)
}

fn read(path: &Path) -> Option<String> {
    fs::read_to_string(path).ok().map(|s| s.trim().to_string())
}

fn read_u32(path: &Path) -> Option<u32> {
    read(path)?.parse().ok()
}

pub fn query_at(root: &Path) -> BrightnessState {
    let Some(dev) = pick(root) else {
        return BrightnessState::default();
    };
    let max = read_u32(&dev.join("max_brightness")).unwrap_or(0);
    let cur = read_u32(&dev.join("brightness")).unwrap_or(0);
    if max == 0 {
        return BrightnessState::default();
    }
    BrightnessState {
        percent: ((cur as u64 * 100 + max as u64 / 2) / max as u64) as u32,
        available: true,
        device: dev
            .file_name()
            .map(|n| n.to_string_lossy().into_owned())
            .unwrap_or_default(),
    }
}

pub fn set_at(root: &Path, percent: u32) -> Result<BrightnessState> {
    let dev = pick(root).ok_or_else(|| anyhow!("no backlight device"))?;
    let max = read_u32(&dev.join("max_brightness")).unwrap_or(0);
    // Never all the way to black: 1% stays readable.
    let pct = percent.clamp(1, 100) as u64;
    let raw = ((pct * max as u64 + 50) / 100).max(1);
    fs::write(dev.join("brightness"), raw.to_string())
        .with_context(|| format!("writing {}", dev.join("brightness").display()))?;
    Ok(query_at(root))
}

pub fn query() -> BrightnessState {
    query_at(Path::new(SYSFS_ROOT))
}

pub fn adjust(delta: i32) -> Result<BrightnessState> {
    let now = query();
    if !now.available {
        return Err(anyhow!("no backlight device"));
    }
    set((now.percent as i32 + delta).clamp(1, 100) as u32)
}

pub fn set(percent: u32) -> Result<BrightnessState> {
    set_at(Path::new(SYSFS_ROOT), percent)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn device(root: &Path, name: &str, kind: &str, cur: u32, max: u32) {
        let d = root.join(name);
        fs::create_dir_all(&d).unwrap();
        fs::write(d.join("type"), kind).unwrap();
        fs::write(d.join("brightness"), cur.to_string()).unwrap();
        fs::write(d.join("max_brightness"), max.to_string()).unwrap();
    }

    #[test]
    fn reads_and_writes_the_preferred_backlight() {
        let dir = tempfile::tempdir().unwrap();
        assert!(!query_at(dir.path()).available);
        device(dir.path(), "amdgpu_bl1", "raw", 100, 255);
        device(dir.path(), "acpi_video0", "firmware", 40, 100);
        let s = query_at(dir.path());
        assert_eq!(s.device, "acpi_video0");
        assert_eq!(s.percent, 40);
        let s = set_at(dir.path(), 0).unwrap();
        assert_eq!(s.percent, 1);
        let s = set_at(dir.path(), 75).unwrap();
        assert_eq!(s.percent, 75);
        assert_eq!(
            fs::read_to_string(dir.path().join("acpi_video0/brightness")).unwrap(),
            "75"
        );
    }
}
