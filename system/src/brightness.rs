//! Backlight control via `brightnessctl`.

use anyhow::Result;

use crate::runner::CommandRunner;

#[derive(Clone, Debug, Default, PartialEq)]
pub struct BrightnessState {
    pub percent: u32,
    pub available: bool,
    pub device: String,
}

pub fn query(r: &dyn CommandRunner) -> BrightnessState {
    // Machine-readable: device,class,current,percent,max
    match r.run("brightnessctl", &["-m", "-c", "backlight"]) {
        Ok(out) if out.ok() => {
            let line = out.stdout.lines().next().unwrap_or("");
            let cols: Vec<&str> = line.split(',').collect();
            if cols.len() >= 4 {
                let pct = cols[3].trim_end_matches('%').parse().unwrap_or(0);
                return BrightnessState {
                    percent: pct,
                    available: true,
                    device: cols[0].to_string(),
                };
            }
            BrightnessState::default()
        }
        _ => BrightnessState::default(),
    }
}

pub fn adjust(r: &dyn CommandRunner, delta: i32) -> Result<BrightnessState> {
    let arg = if delta >= 0 {
        format!("{delta}%+")
    } else {
        format!("{}%-", -delta)
    };
    r.run_ok(
        "brightnessctl",
        &["-q", "-c", "backlight", "-n", "1%", "set", &arg],
    )?;
    Ok(query(r))
}

pub fn set(r: &dyn CommandRunner, percent: u32) -> Result<BrightnessState> {
    r.run_ok(
        "brightnessctl",
        &[
            "-q",
            "-c",
            "backlight",
            "-n",
            "1%",
            "set",
            &format!("{}%", percent.clamp(1, 100)),
        ],
    )?;
    Ok(query(r))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::runner::FakeRunner;

    #[test]
    fn parses_machine_output() {
        let r = FakeRunner::default().with(
            "brightnessctl -m -c backlight",
            "intel_backlight,backlight,3800,40%,9500\n",
        );
        let s = query(&r);
        assert_eq!(s.percent, 40);
        assert_eq!(s.device, "intel_backlight");
    }
}
