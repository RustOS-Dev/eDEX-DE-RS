//! Configuration schema. Every field has a default so partial files load.

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Appearance {
    pub theme: String,
    /// Font family name; empty = default monospace.
    pub font: String,
    pub font_size: f32,
    pub border_glow: f32,
    pub scanlines: bool,
    pub animations: bool,
    pub keyboard_visible: bool,
    pub boot_animation: bool,
}

impl Default for Appearance {
    fn default() -> Self {
        Self { theme: "tron".into(), font: "JetBrainsMono Nerd Font".into(), font_size: 14.0, border_glow: 0.8, scanlines: true, animations: true, keyboard_visible: true, boot_animation: true }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Layout {
    pub fs_split: f32,
    pub sysinfo_split: f32,
    pub reserve_side_panels: bool,
}

impl Default for Layout {
    fn default() -> Self {
        Self { fs_split: 0.20, sysinfo_split: 0.78, reserve_side_panels: true }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Terminal {
    /// Empty = login shell.
    pub shell: String,
    pub scrollback: usize,
    pub font_size: f32,
    /// block | underline | beam
    pub cursor: String,
    pub cursor_blink: bool,
    /// visual | audible | none
    pub bell: String,
    pub osc52_read: bool,
}

impl Default for Terminal {
    fn default() -> Self {
        Self { shell: String::new(), scrollback: 10_000, font_size: 13.0, cursor: "block".into(), cursor_blink: true, bell: "visual".into(), osc52_read: false }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Launcher {
    pub show_hidden: bool,
    pub terminal_command: String,
}

impl Default for Launcher {
    fn default() -> Self {
        Self { show_hidden: false, terminal_command: "kitty -e".into() }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Notifications {
    pub dnd: bool,
    pub timeout_ms: u32,
    /// top-right | top-left | bottom-right | bottom-left
    pub position: String,
    pub max_visible: usize,
    pub muted_apps: Vec<String>,
}

impl Default for Notifications {
    fn default() -> Self {
        Self { dnd: false, timeout_ms: 5000, position: "top-right".into(), max_visible: 4, muted_apps: Vec::new() }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Wm {
    pub gaps_in: u32,
    pub gaps_out: u32,
    pub border: u32,
    /// dwindle | master | scrolling
    pub layout: String,
    pub workspaces: u32,
    pub animations: bool,
    pub blur: bool,
    pub rounding: u32,
}

impl Default for Wm {
    fn default() -> Self {
        Self { gaps_in: 4, gaps_out: 8, border: 2, layout: "dwindle".into(), workspaces: 9, animations: true, blur: false, rounding: 0 }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Input {
    pub kb_layout: String,
    pub kb_variant: String,
    pub kb_options: String,
    pub repeat_rate: u32,
    pub repeat_delay: u32,
    pub natural_scroll: bool,
    pub tap_to_click: bool,
    pub sensitivity: f32,
}

impl Default for Input {
    fn default() -> Self {
        Self { kb_layout: "us".into(), kb_variant: String::new(), kb_options: String::new(), repeat_rate: 30, repeat_delay: 300, natural_scroll: true, tap_to_click: true, sensitivity: 0.0 }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Monitor {
    /// Output name; empty = catch-all rule.
    pub name: String,
    pub mode: String,
    pub position: String,
    pub scale: f32,
    pub transform: u32,
    pub disabled: bool,
}

impl Default for Monitor {
    fn default() -> Self {
        Self { name: String::new(), mode: "preferred".into(), position: "auto".into(), scale: 1.0, transform: 0, disabled: false }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Display {
    pub monitors: Vec<Monitor>,
    pub night_light: bool,
    pub night_temp: u32,
}

impl Default for Display {
    fn default() -> Self {
        Self { monitors: vec![Monitor::default()], night_light: false, night_temp: 4000 }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Power {
    pub dim_after: u32,
    pub lock_after: u32,
    pub dpms_after: u32,
    pub suspend_after: u32,
    /// power-saver | balanced | performance
    pub profile: String,
    /// suspend | ignore | lock | poweroff
    pub lid_close: String,
    pub lock_on_sleep: bool,
}

impl Default for Power {
    fn default() -> Self {
        Self { dim_after: 300, lock_after: 600, dpms_after: 900, suspend_after: 0, profile: "balanced".into(), lid_close: "suspend".into(), lock_on_sleep: true }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Privacy {
    pub tor_mode_on_login: bool,
    pub tailscale_exit_node: String,
    pub fingerprint_login: bool,
}

impl Default for Privacy {
    fn default() -> Self {
        Self { tor_mode_on_login: false, tailscale_exit_node: String::new(), fingerprint_login: true }
    }
}

/// The whole configuration file.
#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct Config {
    pub appearance: Appearance,
    pub layout: Layout,
    pub terminal: Terminal,
    pub launcher: Launcher,
    pub notifications: Notifications,
    pub wm: Wm,
    pub input: Input,
    pub display: Display,
    pub power: Power,
    pub privacy: Privacy,
}

impl Config {
    /// Clamp values into sane ranges after loading or editing.
    pub fn sanitize(&mut self) {
        self.appearance.font_size = self.appearance.font_size.clamp(8.0, 32.0);
        self.appearance.border_glow = self.appearance.border_glow.clamp(0.0, 1.0);
        self.layout.fs_split = self.layout.fs_split.clamp(0.08, 0.5);
        self.layout.sysinfo_split = self.layout.sysinfo_split.clamp(0.5, 0.95);
        self.terminal.font_size = self.terminal.font_size.clamp(8.0, 32.0);
        self.terminal.scrollback = self.terminal.scrollback.clamp(100, 200_000);
        if !["block", "underline", "beam"].contains(&self.terminal.cursor.as_str()) {
            self.terminal.cursor = "block".into();
        }
        self.wm.workspaces = self.wm.workspaces.clamp(1, 10);
        self.wm.gaps_in = self.wm.gaps_in.min(64);
        self.wm.gaps_out = self.wm.gaps_out.min(128);
        self.wm.border = self.wm.border.min(10);
        if !["dwindle", "master", "scrolling"].contains(&self.wm.layout.as_str()) {
            self.wm.layout = "dwindle".into();
        }
        self.input.repeat_rate = self.input.repeat_rate.clamp(1, 200);
        self.input.repeat_delay = self.input.repeat_delay.clamp(100, 2000);
        self.input.sensitivity = self.input.sensitivity.clamp(-1.0, 1.0);
        self.display.night_temp = self.display.night_temp.clamp(1000, 6500);
        if self.display.monitors.is_empty() {
            self.display.monitors.push(Monitor::default());
        }
        for m in &mut self.display.monitors {
            m.scale = if m.scale <= 0.0 { 1.0 } else { m.scale.clamp(0.5, 4.0) };
        }
        if !["power-saver", "balanced", "performance"].contains(&self.power.profile.as_str()) {
            self.power.profile = "balanced".into();
        }
        if !["suspend", "ignore", "lock", "poweroff", "hibernate"].contains(&self.power.lid_close.as_str()) {
            self.power.lid_close = "suspend".into();
        }
        self.notifications.max_visible = self.notifications.max_visible.clamp(1, 10);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn partial_toml_merges_with_defaults() {
        let c: Config = toml::from_str("[appearance]\ntheme = \"matrix\"\n[wm]\ngaps_in = 9\n").unwrap();
        assert_eq!(c.appearance.theme, "matrix");
        assert_eq!(c.appearance.font_size, 14.0);
        assert_eq!(c.wm.gaps_in, 9);
        assert_eq!(c.wm.gaps_out, 8);
    }

    #[test]
    fn roundtrip_and_sanitize() {
        let mut c = Config::default();
        c.appearance.font_size = 900.0;
        c.terminal.cursor = "weird".into();
        c.sanitize();
        assert_eq!(c.appearance.font_size, 32.0);
        assert_eq!(c.terminal.cursor, "block");
        let text = toml::to_string_pretty(&c).unwrap();
        let back: Config = toml::from_str(&text).unwrap();
        assert_eq!(back, c);
    }
}
