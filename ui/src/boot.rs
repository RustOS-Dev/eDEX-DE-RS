//! Boot sequence animation: typewriter lines, then a fade into the shell.

use std::time::{Duration, Instant};

const CHAR_STEP: Duration = Duration::from_millis(6);
const LINE_PAUSE: Duration = Duration::from_millis(90);
const FADE: Duration = Duration::from_millis(500);

pub const BOOT_LINES: &[&str] = &[
    "eDEX-DE SHELL // BOOT SEQUENCE",
    "> connecting to wayland display ............ ok",
    "> binding layer-shell surfaces ............ ok",
    "> initialising gpu renderer ............... ok",
    "> spawning terminal session ............... ok",
    "> mounting filesystem interface ........... ok",
    "> attaching system monitor ................ ok",
    "> handshake with edex-comp ................ ok",
    "> privacy subsystems ...................... armed",
    "SYSTEM READY",
];

#[derive(Clone, Debug)]
pub struct BootAnimation {
    start: Instant,
    line: usize,
    chars: usize,
    last_step: Instant,
    fade_start: Option<Instant>,
    pub done: bool,
    pub enabled: bool,
}

impl Default for BootAnimation {
    fn default() -> Self {
        Self::new(true)
    }
}

impl BootAnimation {
    pub fn new(enabled: bool) -> Self {
        let now = Instant::now();
        Self {
            start: now,
            line: 0,
            chars: 0,
            last_step: now,
            fade_start: None,
            done: !enabled,
            enabled,
        }
    }

    pub fn skip(&mut self) {
        self.done = true;
    }

    /// Advance the animation; returns true when it is finished.
    pub fn update(&mut self, now: Instant) -> bool {
        if self.done {
            return true;
        }
        if let Some(fade) = self.fade_start {
            if now.duration_since(fade) >= FADE {
                self.done = true;
            }
            return self.done;
        }
        while now.duration_since(self.last_step) >= CHAR_STEP {
            self.last_step += CHAR_STEP;
            let current = BOOT_LINES[self.line];
            if self.chars < current.len() {
                self.chars += 1;
            } else if self.line + 1 < BOOT_LINES.len() {
                if now.duration_since(self.last_step) < LINE_PAUSE {
                    break;
                }
                self.last_step = now;
                self.line += 1;
                self.chars = 0;
            } else {
                self.fade_start = Some(now);
                break;
            }
        }
        false
    }

    /// Lines currently displayed (the last one may be partial).
    pub fn lines(&self) -> Vec<String> {
        let mut out: Vec<String> = BOOT_LINES[..self.line]
            .iter()
            .map(|s| s.to_string())
            .collect();
        let current = BOOT_LINES[self.line];
        out.push(current.chars().take(self.chars).collect());
        out
    }

    /// Opacity of the boot overlay (1 while typing, fading to 0).
    pub fn overlay_alpha(&self, now: Instant) -> f32 {
        if self.done {
            return 0.0;
        }
        match self.fade_start {
            Some(fade) => {
                1.0 - (now.duration_since(fade).as_secs_f32() / FADE.as_secs_f32()).clamp(0.0, 1.0)
            }
            None => 1.0,
        }
    }

    pub fn elapsed(&self, now: Instant) -> Duration {
        now.duration_since(self.start)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn boot_finishes() {
        let mut anim = BootAnimation::new(true);
        let mut now = Instant::now();
        let mut steps = 0;
        while !anim.update(now) {
            now += Duration::from_millis(20);
            steps += 1;
            assert!(steps < 10_000);
        }
        assert!(anim.done);
        assert_eq!(anim.overlay_alpha(now), 0.0);
        let disabled = BootAnimation::new(false);
        assert!(disabled.done);
    }
}
