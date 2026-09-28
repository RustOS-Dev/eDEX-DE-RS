//! PipeWire audio via `wpctl`.

use anyhow::Result;

use crate::runner::CommandRunner;

#[derive(Clone, Debug, Default, PartialEq)]
pub struct AudioDevice {
    pub id: u32,
    pub name: String,
    pub default: bool,
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct AudioState {
    pub volume: u32,
    pub muted: bool,
    pub mic_volume: u32,
    pub mic_muted: bool,
    pub sinks: Vec<AudioDevice>,
    pub sources: Vec<AudioDevice>,
    pub available: bool,
}

fn parse_volume(s: &str) -> Option<(u32, bool)> {
    // "Volume: 0.42 [MUTED]"
    let rest = s.trim().strip_prefix("Volume:")?.trim();
    let mut parts = rest.split_whitespace();
    let v: f32 = parts.next()?.parse().ok()?;
    let muted = rest.contains("[MUTED]");
    Some(((v * 100.0).round().clamp(0.0, 150.0) as u32, muted))
}

/// Parse the Sinks/Sources sections of `wpctl status`.
fn parse_status(text: &str) -> (Vec<AudioDevice>, Vec<AudioDevice>) {
    let mut sinks = Vec::new();
    let mut sources = Vec::new();
    let mut section = "";
    for line in text.lines() {
        let l = line.trim_start_matches(['│', '├', '└', '─', ' ']).trim();
        if l.starts_with("Sinks:") {
            section = "sinks";
            continue;
        } else if l.starts_with("Sources:") {
            section = "sources";
            continue;
        } else if l.ends_with(':')
            && !l.starts_with('*')
            && !l.chars().next().is_some_and(|c| c.is_ascii_digit())
        {
            section = "";
            continue;
        }
        if section.is_empty() || l.is_empty() {
            continue;
        }
        let default = l.starts_with('*');
        let l = l.trim_start_matches('*').trim();
        let Some((id, rest)) = l.split_once('.') else {
            continue;
        };
        let Ok(id) = id.trim().parse::<u32>() else {
            continue;
        };
        let name = rest.split('[').next().unwrap_or("").trim().to_string();
        let dev = AudioDevice { id, name, default };
        if section == "sinks" {
            sinks.push(dev);
        } else {
            sources.push(dev);
        }
    }
    (sinks, sources)
}

pub fn query(r: &dyn CommandRunner) -> AudioState {
    let mut state = AudioState::default();
    match r.run("wpctl", &["get-volume", "@DEFAULT_AUDIO_SINK@"]) {
        Ok(out) if out.ok() => {
            if let Some((v, m)) = parse_volume(&out.stdout) {
                state.volume = v;
                state.muted = m;
                state.available = true;
            }
        }
        _ => return state,
    }
    if let Ok(out) = r.run("wpctl", &["get-volume", "@DEFAULT_AUDIO_SOURCE@"]) {
        if let Some((v, m)) = parse_volume(&out.stdout) {
            state.mic_volume = v;
            state.mic_muted = m;
        }
    }
    if let Ok(out) = r.run("wpctl", &["status"]) {
        let (sinks, sources) = parse_status(&out.stdout);
        state.sinks = sinks;
        state.sources = sources;
    }
    state
}

pub fn set_volume(r: &dyn CommandRunner, percent: u32) -> Result<()> {
    r.run_ok(
        "wpctl",
        &[
            "set-volume",
            "-l",
            "1.0",
            "@DEFAULT_AUDIO_SINK@",
            &format!("{}%", percent.min(100)),
        ],
    )
    .map(|_| ())
}

pub fn adjust_volume(r: &dyn CommandRunner, delta: i32) -> Result<()> {
    let arg = if delta >= 0 {
        format!("{delta}%+")
    } else {
        format!("{}%-", -delta)
    };
    r.run_ok(
        "wpctl",
        &["set-volume", "-l", "1.0", "@DEFAULT_AUDIO_SINK@", &arg],
    )
    .map(|_| ())
}

pub fn toggle_mute(r: &dyn CommandRunner) -> Result<()> {
    r.run_ok("wpctl", &["set-mute", "@DEFAULT_AUDIO_SINK@", "toggle"])
        .map(|_| ())
}

pub fn toggle_mic_mute(r: &dyn CommandRunner) -> Result<()> {
    r.run_ok("wpctl", &["set-mute", "@DEFAULT_AUDIO_SOURCE@", "toggle"])
        .map(|_| ())
}

pub fn set_default(r: &dyn CommandRunner, id: u32) -> Result<()> {
    r.run_ok("wpctl", &["set-default", &id.to_string()])
        .map(|_| ())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::runner::FakeRunner;

    #[test]
    fn parses_wpctl() {
        let status = "PipeWire 'pipewire-0' [1.2.0]\nAudio\n ├─ Devices:\n │      44. Built-in Audio [alsa]\n │  \n ├─ Sinks:\n │  *   49. Built-in Audio Analog Stereo [vol: 0.42]\n │      52. HDMI Output [vol: 1.00 MUTED]\n │  \n ├─ Sources:\n │  *   50. Built-in Audio Mic [vol: 0.80]\n │  \nVideo\n ├─ Devices:\n";
        let r = FakeRunner::default()
            .with("wpctl get-volume @DEFAULT_AUDIO_SINK@", "Volume: 0.42\n")
            .with(
                "wpctl get-volume @DEFAULT_AUDIO_SOURCE@",
                "Volume: 0.80 [MUTED]\n",
            )
            .with("wpctl status", status);
        let s = query(&r);
        assert!(s.available);
        assert_eq!(s.volume, 42);
        assert!(s.mic_muted);
        assert_eq!(s.sinks.len(), 2);
        assert!(s.sinks[0].default);
        assert_eq!(s.sinks[1].name, "HDMI Output");
        assert_eq!(s.sources[0].id, 50);
        adjust_volume(&r, -5).unwrap_err();
        assert!(r.calls().iter().any(|c| c.contains("5%-")));
    }
}
