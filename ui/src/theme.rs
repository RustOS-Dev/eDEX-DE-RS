//! Theme definitions loaded from TOML (`/usr/share/edex-de/themes`, `~/.config/edex-de/themes`).

use std::{collections::BTreeMap, fs, path::Path};

use anyhow::{anyhow, Context, Result};
use serde::{Deserialize, Serialize};

use crate::geometry::{mix, parse_color, with_alpha, Color};

pub const TRON_TOML: &str = include_str!("../../themes/tron.toml");

/// Fully resolved theme colours.
#[derive(Clone, Debug, PartialEq)]
pub struct Theme {
    pub name: String,
    pub background: Color,
    pub panel_bg: Color,
    pub border: Color,
    pub border_glow: Color,
    pub text_primary: Color,
    pub text_secondary: Color,
    pub text_dim: Color,
    pub accent: Color,
    pub warning: Color,
    pub error: Color,
    pub cursor: Color,
    pub selection: Color,
    pub terminal_fg: Color,
    pub terminal_bg: Color,
    pub palette: [Color; 16],
    pub key_fill: Color,
    pub key_active: Color,
    pub key_border: Color,
    /// Border glow strength 0..=1.
    pub glow: f32,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct TerminalColors {
    #[serde(default)]
    pub fg: Option<String>,
    #[serde(default)]
    pub bg: Option<String>,
    #[serde(default)]
    pub palette: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct KeyboardColors {
    #[serde(default)]
    pub key: Option<String>,
    #[serde(default)]
    pub key_active: Option<String>,
    #[serde(default)]
    pub key_border: Option<String>,
}

/// On-disk theme schema. Only the seven base keys are required; everything else is derived.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ThemeConfig {
    pub name: String,
    pub background: String,
    pub panel_bg: String,
    pub border: String,
    pub text_primary: String,
    pub text_secondary: String,
    pub accent: String,
    #[serde(default)]
    pub warning: Option<String>,
    #[serde(default)]
    pub error: Option<String>,
    #[serde(default)]
    pub cursor: Option<String>,
    #[serde(default)]
    pub selection: Option<String>,
    #[serde(default)]
    pub glow: Option<f32>,
    #[serde(default)]
    pub terminal: TerminalColors,
    #[serde(default)]
    pub keyboard: KeyboardColors,
}

fn color(field: &str, value: &str) -> Result<Color> {
    parse_color(value)
        .ok_or_else(|| anyhow!("theme field `{field}` is not a #rrggbb colour: {value}"))
}

fn opt_color(field: &str, value: &Option<String>, fallback: Color) -> Result<Color> {
    match value {
        Some(v) => color(field, v),
        None => Ok(fallback),
    }
}

const DEFAULT_PALETTE: [&str; 16] = [
    "#0a0e1a", "#ff4444", "#00ff88", "#ffd866", "#3d8bff", "#c678dd", "#00d4ff", "#c8e6ff",
    "#4a6a8a", "#ff7a7a", "#66ffb3", "#ffe89a", "#7fb4ff", "#e0a4ff", "#7fe9ff", "#ffffff",
];

impl ThemeConfig {
    pub fn to_theme(&self) -> Result<Theme> {
        let border = color("border", &self.border)?;
        let text_secondary = color("text_secondary", &self.text_secondary)?;
        let background = color("background", &self.background)?;
        let text_primary = color("text_primary", &self.text_primary)?;
        let accent = color("accent", &self.accent)?;
        let mut palette = [background; 16];
        for (i, slot) in palette.iter_mut().enumerate() {
            let value = self
                .terminal
                .palette
                .get(i)
                .map(String::as_str)
                .unwrap_or(DEFAULT_PALETTE[i]);
            *slot = color("terminal.palette", value)?;
        }
        Ok(Theme {
            name: self.name.clone(),
            background,
            panel_bg: color("panel_bg", &self.panel_bg)?,
            border,
            border_glow: with_alpha(border, 0.3),
            text_primary,
            text_secondary,
            text_dim: with_alpha(text_secondary, 0.5),
            accent,
            warning: opt_color("warning", &self.warning, [1.0, 0.624, 0.0, 1.0])?,
            error: opt_color("error", &self.error, [1.0, 0.267, 0.267, 1.0])?,
            cursor: opt_color("cursor", &self.cursor, border)?,
            selection: opt_color("selection", &self.selection, with_alpha(border, 0.35))?,
            terminal_fg: opt_color("terminal.fg", &self.terminal.fg, text_primary)?,
            terminal_bg: opt_color("terminal.bg", &self.terminal.bg, background)?,
            palette,
            key_fill: opt_color(
                "keyboard.key",
                &self.keyboard.key,
                mix(background, border, 0.12),
            )?,
            key_active: opt_color(
                "keyboard.key_active",
                &self.keyboard.key_active,
                with_alpha(border, 0.85),
            )?,
            key_border: opt_color(
                "keyboard.key_border",
                &self.keyboard.key_border,
                with_alpha(border, 0.6),
            )?,
            glow: self.glow.unwrap_or(0.8).clamp(0.0, 1.0),
        })
    }
}

pub fn parse_theme(toml_str: &str) -> Result<Theme> {
    toml::from_str::<ThemeConfig>(toml_str)
        .context("failed to parse theme TOML")?
        .to_theme()
}

pub fn load_theme_file(path: &Path) -> Result<Theme> {
    let text = fs::read_to_string(path).with_context(|| format!("reading {}", path.display()))?;
    parse_theme(&text).with_context(|| format!("in theme {}", path.display()))
}

/// The built-in fallback theme (always available).
pub fn builtin_tron() -> Theme {
    parse_theme(TRON_TOML).expect("bundled tron theme is valid")
}

/// Load every `*.toml` theme from the given directories; later directories override earlier
/// ones for the same theme name. The bundled Tron theme is always present.
pub fn load_themes(dirs: &[&Path]) -> BTreeMap<String, Theme> {
    let mut themes = BTreeMap::new();
    let tron = builtin_tron();
    themes.insert(tron.name.to_ascii_lowercase(), tron);
    for dir in dirs {
        let Ok(entries) = fs::read_dir(dir) else {
            continue;
        };
        let mut paths: Vec<_> = entries
            .flatten()
            .map(|e| e.path())
            .filter(|p| p.extension().is_some_and(|e| e == "toml"))
            .collect();
        paths.sort();
        for path in paths {
            match load_theme_file(&path) {
                Ok(theme) => {
                    themes.insert(theme.name.to_ascii_lowercase(), theme);
                }
                Err(e) => tracing::warn!("skipping theme {}: {e:#}", path.display()),
            }
        }
    }
    themes
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn bundled_tron_parses() {
        let t = builtin_tron();
        assert_eq!(t.name.to_ascii_lowercase(), "tron");
        assert!(t.palette.iter().all(|c| c[3] > 0.0));
        assert!(t.glow > 0.0);
    }

    #[test]
    fn minimal_theme_derives_the_rest() {
        let t = parse_theme(
            r##"name = "x"
background = "#000000"
panel_bg = "#101010"
border = "#00ff00"
text_primary = "#ffffff"
text_secondary = "#888888"
accent = "#ff00ff"
"##,
        )
        .unwrap();
        assert_eq!(t.cursor, t.border);
        assert_eq!(t.terminal_fg, t.text_primary);
        assert_eq!(t.palette[1], parse_color("#ff4444").unwrap());
    }
}
