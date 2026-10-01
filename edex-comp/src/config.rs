//! What edex-comp takes from the shared `~/.config/edex-de/config.toml`: tiling, input, output
//! rules, idle timeouts, the lid action, night light, the border colour of the theme and the key
//! bindings. The shell's settings panel writes the file; edex-comp watches it and re-applies.

use std::path::{Path, PathBuf};

use crate::{
    binds::{self, Bind},
    wm::{TileLayout, WmConfig},
};

#[derive(Clone, Debug, PartialEq)]
pub struct XkbSettings {
    pub layout: String,
    pub variant: String,
    pub options: String,
    pub repeat_rate: i32,
    pub repeat_delay: i32,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct PointerSettings {
    pub natural_scroll: bool,
    pub tap_to_click: bool,
    /// libinput acceleration speed, -1..=1.
    pub accel_speed: f64,
}

/// An output mode request.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum ModeRule {
    Preferred,
    Exact {
        w: i32,
        h: i32,
        refresh_hz: Option<f32>,
    },
}

#[derive(Clone, Debug, PartialEq)]
pub struct OutputRule {
    /// Empty matches every output without a rule of its own.
    pub name: String,
    pub mode: ModeRule,
    /// `None` = place automatically, to the right of the previous output.
    pub position: Option<(i32, i32)>,
    pub scale: f64,
    pub transform: u32,
    pub disabled: bool,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum LidAction {
    Ignore,
    Lock,
    /// Turn the internal panel off (and lock) — RustOS has no suspend.
    Suspend,
    PowerOff,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct IdleSettings {
    /// Seconds of inactivity before each step; 0 disables it.
    pub lock_after: u32,
    pub dpms_after: u32,
    pub lock_on_lid: bool,
}

#[derive(Clone, Debug)]
pub struct CompConfig {
    pub wm: WmConfig,
    pub border: i32,
    /// Theme accent for the focused window's border and the inactive border, RGBA 0..=1.
    pub border_active: [f32; 4],
    pub border_inactive: [f32; 4],
    pub background: [f32; 4],
    pub xkb: XkbSettings,
    pub pointer: PointerSettings,
    pub outputs: Vec<OutputRule>,
    pub night_light: Option<u32>,
    pub idle: IdleSettings,
    pub lid: LidAction,
    pub binds: Vec<Bind>,
    pub bind_errors: Vec<String>,
    pub terminal: String,
    pub source: PathBuf,
}

impl CompConfig {
    pub fn from_settings(c: &settings::Config, source: &Path) -> Self {
        let theme = load_theme(&c.appearance.theme);
        let (binds, bind_errors) = binds::with_user(&c.wm.binds);
        CompConfig {
            wm: WmConfig {
                gaps_in: c.wm.gaps_in.min(200) as i32,
                gaps_out: c.wm.gaps_out.min(200) as i32,
                layout: TileLayout::from_name(&c.wm.layout),
                workspaces: c.wm.workspaces.clamp(1, 10) as i32,
            },
            border: c.wm.border.min(20) as i32,
            border_active: theme.accent,
            border_inactive: theme.border,
            background: theme.background,
            xkb: XkbSettings {
                layout: if c.input.kb_layout.is_empty() {
                    "us".into()
                } else {
                    c.input.kb_layout.clone()
                },
                variant: c.input.kb_variant.clone(),
                options: c.input.kb_options.clone(),
                repeat_rate: c.input.repeat_rate.clamp(1, 100) as i32,
                repeat_delay: c.input.repeat_delay.clamp(100, 2000) as i32,
            },
            pointer: PointerSettings {
                natural_scroll: c.input.natural_scroll,
                tap_to_click: c.input.tap_to_click,
                accel_speed: c.input.sensitivity.clamp(-1.0, 1.0) as f64,
            },
            outputs: c.display.monitors.iter().map(output_rule).collect(),
            night_light: c
                .display
                .night_light
                .then_some(c.display.night_temp.clamp(1000, 10000)),
            idle: IdleSettings {
                lock_after: c.power.lock_after,
                dpms_after: c.power.dpms_after,
                lock_on_lid: c.power.lock_on_sleep,
            },
            lid: match c.power.lid_close.as_str() {
                "ignore" => LidAction::Ignore,
                "lock" => LidAction::Lock,
                "poweroff" => LidAction::PowerOff,
                _ => LidAction::Suspend,
            },
            binds,
            bind_errors,
            terminal: c.launcher.terminal_command.clone(),
            source: source.to_path_buf(),
        }
    }

    /// Load the config of the user whose home is `home` (the session user), or the defaults.
    pub fn load(path: &Path) -> Self {
        let c = if path.exists() {
            settings::load(path)
        } else {
            settings::Config::default()
        };
        Self::from_settings(&c, path)
    }

    /// The rule for an output: its own, else the catch-all, else defaults.
    pub fn output_rule(&self, name: &str) -> OutputRule {
        self.outputs
            .iter()
            .find(|r| r.name == name)
            .or_else(|| self.outputs.iter().find(|r| r.name.is_empty()))
            .cloned()
            .unwrap_or(OutputRule {
                name: String::new(),
                mode: ModeRule::Preferred,
                position: None,
                scale: 1.0,
                transform: 0,
                disabled: false,
            })
    }
}

fn load_theme(name: &str) -> ui::theme::Theme {
    let share = settings::system_share_dir().join("themes");
    let user = settings::user_theme_dir();
    let themes = ui::theme::load_themes(&[share.as_path(), user.as_path()]);
    themes
        .get(&name.to_ascii_lowercase())
        .cloned()
        .unwrap_or_else(ui::theme::builtin_tron)
}

fn output_rule(m: &settings::Monitor) -> OutputRule {
    OutputRule {
        name: m.name.clone(),
        mode: parse_mode(&m.mode),
        position: parse_position(&m.position),
        scale: if m.scale > 0.0 {
            (m.scale as f64).clamp(0.5, 4.0)
        } else {
            1.0
        },
        transform: m.transform.min(7),
        disabled: m.disabled,
    }
}

/// `preferred`, `1920x1080` or `2560x1440@144`.
pub fn parse_mode(s: &str) -> ModeRule {
    let s = s.trim();
    let (size, refresh) = match s.split_once('@') {
        Some((a, b)) => (a, b.trim_end_matches("Hz").parse::<f32>().ok()),
        None => (s, None),
    };
    match size.split_once('x') {
        Some((w, h)) => match (w.trim().parse(), h.trim().parse()) {
            (Ok(w), Ok(h)) if w > 0 && h > 0 => ModeRule::Exact {
                w,
                h,
                refresh_hz: refresh,
            },
            _ => ModeRule::Preferred,
        },
        None => ModeRule::Preferred,
    }
}

/// `auto`, `XxY` or `X,Y`.
pub fn parse_position(s: &str) -> Option<(i32, i32)> {
    let s = s.trim();
    let (x, y) = s.split_once(',').or_else(|| s.split_once('x'))?;
    Some((x.trim().parse().ok()?, y.trim().parse().ok()?))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn modes_and_positions() {
        assert_eq!(parse_mode("preferred"), ModeRule::Preferred);
        assert_eq!(
            parse_mode("2560x1440@144"),
            ModeRule::Exact {
                w: 2560,
                h: 1440,
                refresh_hz: Some(144.0)
            }
        );
        assert_eq!(
            parse_mode("1920x1080"),
            ModeRule::Exact {
                w: 1920,
                h: 1080,
                refresh_hz: None
            }
        );
        assert_eq!(parse_mode("0x0"), ModeRule::Preferred);
        assert_eq!(parse_position("1920x0"), Some((1920, 0)));
        assert_eq!(parse_position("-1280, 0"), Some((-1280, 0)));
        assert_eq!(parse_position("auto"), None);
    }

    #[test]
    fn derives_from_settings() {
        let mut s = settings::Config::default();
        s.wm.layout = "master".into();
        s.wm.workspaces = 42;
        s.display.night_light = true;
        s.display.night_temp = 3500;
        s.power.lid_close = "lock".into();
        s.display.monitors = vec![
            settings::Monitor {
                name: "DP-1".into(),
                mode: "2560x1440@144".into(),
                position: "0x0".into(),
                scale: 1.25,
                transform: 0,
                disabled: false,
            },
            settings::Monitor::default(),
        ];
        let c = CompConfig::from_settings(&s, Path::new("/x/config.toml"));
        assert_eq!(c.wm.layout, TileLayout::Master);
        assert_eq!(c.wm.workspaces, 10);
        assert_eq!(c.night_light, Some(3500));
        assert_eq!(c.lid, LidAction::Lock);
        assert_eq!(c.output_rule("DP-1").scale, 1.25);
        assert_eq!(c.output_rule("HDMI-A-1").mode, ModeRule::Preferred);
        assert!(!c.binds.is_empty());
    }
}
