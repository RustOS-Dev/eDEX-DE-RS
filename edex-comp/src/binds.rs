//! Key bindings: the built-in eDEX set (the former Hyprland `binds.lua`) plus the user's
//! `[[wm.binds]]` entries from `config.toml`.
//!
//! Keys are written `MODS+Key`, e.g. `SUPER+SHIFT+Return`, `XF86AudioMute` or `Print`. Keysyms
//! match on the unshifted symbol of the key, so `SUPER+SHIFT+1` means the key labelled 1.

use comp_proto::{Direction, WorkspaceTarget, SCRATCH_WORKSPACE};
use smithay::input::keyboard::{keysyms, xkb, Keysym, ModifiersState};

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub struct Mods {
    pub logo: bool,
    pub shift: bool,
    pub ctrl: bool,
    pub alt: bool,
}

impl From<&ModifiersState> for Mods {
    fn from(m: &ModifiersState) -> Self {
        Self {
            logo: m.logo,
            shift: m.shift,
            ctrl: m.ctrl,
            alt: m.alt,
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Action {
    /// `shell ARGS…`: an `edex-de ipc` request to the shell (overlays, OSD, focus).
    Shell(Vec<String>),
    /// `exec CMD`: start a program in the session.
    Exec(String),
    Close,
    Kill,
    Minimize,
    ToggleMaximize,
    ToggleFullscreen,
    ToggleFloat,
    CenterFloat,
    Focus(Direction),
    Move(Direction),
    Cycle {
        reverse: bool,
    },
    Workspace(WorkspaceTarget),
    MoveToWorkspace {
        target: WorkspaceTarget,
        follow: bool,
    },
    ToggleScratch,
    Lock,
    Exit,
    Reload,
    /// `screenshot output|region` — `output` saves the focused output to
    /// `~/Pictures/Screenshots`; `region` hands over to `slurp`/`grim`.
    Screenshot {
        region: bool,
    },
    /// Switch to virtual terminal N (only on the udev backend).
    VtSwitch(i32),
    /// Removes a built-in binding.
    None,
}

impl Action {
    pub fn parse(s: &str) -> Result<Action, String> {
        let s = s.trim();
        let (verb, rest) = s.split_once(char::is_whitespace).unwrap_or((s, ""));
        let rest = rest.trim();
        let dir = |r: &str| -> Result<Direction, String> {
            match r {
                "left" => Ok(Direction::Left),
                "right" => Ok(Direction::Right),
                "up" => Ok(Direction::Up),
                "down" => Ok(Direction::Down),
                o => Err(format!("unknown direction `{o}` (left|right|up|down)")),
            }
        };
        let ws = |r: &str| -> Result<WorkspaceTarget, String> {
            match r {
                "next" | "e+1" => Ok(WorkspaceTarget::Next),
                "prev" | "e-1" => Ok(WorkspaceTarget::Prev),
                "empty" => Ok(WorkspaceTarget::Empty),
                "scratch" => Ok(WorkspaceTarget::Id(SCRATCH_WORKSPACE)),
                n => n
                    .parse()
                    .map(WorkspaceTarget::Id)
                    .map_err(|_| format!("unknown workspace `{n}`")),
            }
        };
        Ok(match verb {
            "shell" if !rest.is_empty() => {
                Action::Shell(rest.split_whitespace().map(str::to_string).collect())
            }
            "exec" if !rest.is_empty() => Action::Exec(rest.to_string()),
            "close" => Action::Close,
            "kill" => Action::Kill,
            "minimize" => Action::Minimize,
            "maximize" => Action::ToggleMaximize,
            "fullscreen" => Action::ToggleFullscreen,
            "float" => Action::ToggleFloat,
            "center" => Action::CenterFloat,
            "focus" => Action::Focus(dir(rest)?),
            "move" => Action::Move(dir(rest)?),
            "cycle" => Action::Cycle {
                reverse: rest == "prev",
            },
            "workspace" => Action::Workspace(ws(rest)?),
            "movetoworkspace" | "move-to-workspace" => {
                let (target, flag) = rest.split_once(' ').unwrap_or((rest, ""));
                Action::MoveToWorkspace {
                    target: ws(target)?,
                    follow: flag.trim() == "follow",
                }
            }
            "scratch" => Action::ToggleScratch,
            "lock" => Action::Lock,
            "exit" => Action::Exit,
            "reload" => Action::Reload,
            "screenshot" => Action::Screenshot {
                region: rest == "region",
            },
            "vt" => Action::VtSwitch(rest.parse().map_err(|_| format!("bad vt `{rest}`"))?),
            "none" => Action::None,
            _ => return Err(format!("unknown action `{s}`")),
        })
    }
}

/// Lower-case Latin and Latin-1 letter keysyms (they share their code points with Unicode).
fn to_lower(sym: Keysym) -> Keysym {
    let raw = sym.raw();
    match raw {
        0x41..=0x5a | 0xc0..=0xd6 | 0xd8..=0xde => Keysym::from(raw + 0x20),
        _ => sym,
    }
}

/// What a binding listens for.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Trigger {
    Key(Keysym),
    /// Tapping a modifier key on its own (pressed and released with nothing in between).
    Tap(Keysym),
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Bind {
    pub mods: Mods,
    pub trigger: Trigger,
    pub action: Action,
    /// Fires while the session is locked (media keys).
    pub locked: bool,
    /// Repeats while held.
    pub repeat: bool,
}

/// Parse `SUPER+SHIFT+Return` (a trailing `:tap` makes a lone-modifier tap binding).
pub fn parse_keys(keys: &str) -> Result<(Mods, Trigger), String> {
    let (keys, tap) = match keys.trim().strip_suffix(":tap") {
        Some(k) => (k, true),
        None => (keys.trim(), false),
    };
    let mut mods = Mods::default();
    let parts: Vec<&str> = keys.split('+').map(str::trim).collect();
    let (key, modifiers) = parts.split_last().ok_or("empty binding")?;
    for m in modifiers {
        match m.to_ascii_uppercase().as_str() {
            "SUPER" | "MOD4" | "LOGO" | "WIN" => mods.logo = true,
            "SHIFT" => mods.shift = true,
            "CTRL" | "CONTROL" => mods.ctrl = true,
            "ALT" | "MOD1" => mods.alt = true,
            other => return Err(format!("unknown modifier `{other}`")),
        }
    }
    let mut sym = xkb::keysym_from_name(key, xkb::KEYSYM_NO_FLAGS);
    if sym.raw() == keysyms::KEY_NoSymbol {
        sym = xkb::keysym_from_name(key, xkb::KEYSYM_CASE_INSENSITIVE);
    }
    if sym.raw() == keysyms::KEY_NoSymbol {
        return Err(format!("unknown key `{key}`"));
    }
    // Letters are matched in lower case: the unshifted symbol of the key.
    let sym = to_lower(sym);
    Ok((
        mods,
        if tap {
            Trigger::Tap(sym)
        } else {
            Trigger::Key(sym)
        },
    ))
}

fn bind(keys: &str, action: &str) -> Bind {
    let (mods, trigger) = parse_keys(keys).expect("built-in binding keys");
    Bind {
        mods,
        trigger,
        action: Action::parse(action).expect("built-in binding action"),
        locked: false,
        repeat: false,
    }
}

/// The built-in bindings (same keys as the eDEX-OS Hyprland config).
pub fn defaults() -> Vec<Bind> {
    let mut b = vec![
        bind("SUPER+Super_L:tap", "shell toggle launcher"),
        bind("SUPER+space", "shell toggle launcher"),
        bind("SUPER+comma", "shell toggle settings"),
        bind("SUPER+P", "shell toggle privacy"),
        bind("SUPER+N", "shell toggle notifications"),
        bind("SUPER+Escape", "shell toggle power"),
        bind("SUPER+Return", "shell focus terminal"),
        bind("SUPER+F1", "shell focus filesystem"),
        bind("SUPER+CTRL+F", "shell action side-panels"),
        bind("SUPER+SHIFT+Return", "exec foot"),
        bind("SUPER+B", "exec xdg-open https://"),
        bind("SUPER+Q", "close"),
        bind("SUPER+SHIFT+Q", "kill"),
        bind("SUPER+F", "maximize"),
        bind("SUPER+SHIFT+F", "fullscreen"),
        bind("SUPER+M", "minimize"),
        bind("SUPER+V", "float"),
        bind("SUPER+C", "center"),
        bind("SUPER+Tab", "cycle next"),
        bind("SUPER+SHIFT+Tab", "cycle prev"),
        bind("SUPER+S", "scratch"),
        bind("SUPER+SHIFT+S", "screenshot region"),
        bind("SUPER+bracketright", "workspace next"),
        bind("SUPER+bracketleft", "workspace prev"),
        bind("SUPER+ALT+L", "lock"),
        bind("SUPER+SHIFT+R", "reload"),
        bind("Print", "screenshot output"),
    ];
    for (key, dir) in [
        ("H", "left"),
        ("L", "right"),
        ("K", "up"),
        ("Left", "left"),
        ("Right", "right"),
        ("Up", "up"),
        ("Down", "down"),
    ] {
        b.push(bind(&format!("SUPER+{key}"), &format!("focus {dir}")));
    }
    for (key, dir) in [
        ("H", "left"),
        ("L", "right"),
        ("K", "up"),
        ("J", "down"),
        ("Left", "left"),
        ("Right", "right"),
        ("Up", "up"),
        ("Down", "down"),
    ] {
        b.push(bind(&format!("SUPER+SHIFT+{key}"), &format!("move {dir}")));
    }
    for i in 1..=10 {
        let key = i % 10;
        b.push(bind(&format!("SUPER+{key}"), &format!("workspace {i}")));
        b.push(bind(
            &format!("SUPER+SHIFT+{key}"),
            &format!("movetoworkspace {i}"),
        ));
        b.push(bind(
            &format!("SUPER+CTRL+{key}"),
            &format!("movetoworkspace {i} follow"),
        ));
    }
    for vt in 1..=12 {
        b.push(bind(&format!("CTRL+ALT+F{vt}"), &format!("vt {vt}")));
    }
    // Media keys go through the shell so it shows the OSD; they work on the lock screen.
    for (key, action, repeat) in [
        ("XF86AudioRaiseVolume", "shell audio volume +5", true),
        ("XF86AudioLowerVolume", "shell audio volume -5", true),
        ("XF86AudioMute", "shell audio mute", false),
        ("XF86AudioMicMute", "shell audio mic-mute", false),
        ("XF86MonBrightnessUp", "shell brightness +5", true),
        ("XF86MonBrightnessDown", "shell brightness -5", true),
        ("XF86AudioNext", "exec playerctl next", false),
        ("XF86AudioPause", "exec playerctl play-pause", false),
        ("XF86AudioPlay", "exec playerctl play-pause", false),
        ("XF86AudioPrev", "exec playerctl previous", false),
        ("XF86PowerOff", "shell toggle power", false),
    ] {
        let mut m = bind(key, action);
        m.locked = true;
        m.repeat = repeat;
        b.push(m);
    }
    b
}

/// The built-in set with the user's bindings applied over it. Invalid entries are reported and
/// skipped.
pub fn with_user(user: &[settings::Bind]) -> (Vec<Bind>, Vec<String>) {
    let mut binds = defaults();
    let mut errors = Vec::new();
    for u in user {
        let parsed = parse_keys(&u.keys).and_then(|(mods, trigger)| {
            Action::parse(&u.action).map(|action| (mods, trigger, action))
        });
        match parsed {
            Ok((mods, trigger, action)) => {
                binds.retain(|b| !(b.mods == mods && b.trigger == trigger));
                if action != Action::None {
                    binds.push(Bind {
                        mods,
                        trigger,
                        action,
                        locked: false,
                        repeat: false,
                    });
                }
            }
            Err(e) => errors.push(format!("{} = {}: {e}", u.keys, u.action)),
        }
    }
    (binds, errors)
}

/// `SUPER+SHIFT+Return → exec foot`.
pub fn describe(b: &Bind) -> String {
    let mut keys = String::new();
    for (on, name) in [
        (b.mods.logo, "SUPER+"),
        (b.mods.ctrl, "CTRL+"),
        (b.mods.alt, "ALT+"),
        (b.mods.shift, "SHIFT+"),
    ] {
        if on {
            keys.push_str(name);
        }
    }
    let (sym, tap) = match b.trigger {
        Trigger::Key(s) => (s, ""),
        Trigger::Tap(s) => (s, " (tap)"),
    };
    keys.push_str(&xkb::keysym_get_name(sym));
    format!("{keys}{tap} → {}", describe_action(&b.action))
}

fn describe_action(a: &Action) -> String {
    match a {
        Action::Shell(args) => format!("shell {}", args.join(" ")),
        Action::Exec(cmd) => format!("exec {cmd}"),
        Action::Focus(d) => format!("focus {d:?}").to_lowercase(),
        Action::Move(d) => format!("move {d:?}").to_lowercase(),
        Action::Workspace(t) => format!("workspace {}", target_name(*t)),
        Action::MoveToWorkspace { target, follow } => format!(
            "movetoworkspace {}{}",
            target_name(*target),
            if *follow { " follow" } else { "" }
        ),
        Action::Cycle { reverse } => format!("cycle {}", if *reverse { "prev" } else { "next" }),
        Action::Screenshot { region } => {
            format!("screenshot {}", if *region { "region" } else { "output" })
        }
        Action::VtSwitch(n) => format!("vt {n}"),
        other => format!("{other:?}").to_lowercase(),
    }
}

fn target_name(t: WorkspaceTarget) -> String {
    match t {
        WorkspaceTarget::Id(SCRATCH_WORKSPACE) => "scratch".into(),
        WorkspaceTarget::Id(n) => n.to_string(),
        WorkspaceTarget::Next => "next".into(),
        WorkspaceTarget::Prev => "prev".into(),
        WorkspaceTarget::Empty => "empty".into(),
    }
}

/// The binding for a key press, if any.
pub fn lookup<'a>(binds: &'a [Bind], mods: Mods, raw_syms: &[Keysym]) -> Option<&'a Bind> {
    binds.iter().find(|b| {
        b.mods == mods
            && match b.trigger {
                Trigger::Key(sym) => raw_syms.iter().any(|s| to_lower(*s) == sym),
                Trigger::Tap(_) => false,
            }
    })
}

/// The tap binding for a lone modifier key that was just released.
pub fn lookup_tap(binds: &[Bind], sym: Keysym) -> Option<&Bind> {
    binds
        .iter()
        .find(|b| matches!(b.trigger, Trigger::Tap(s) if s == sym))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_keys_and_actions() {
        let (m, t) = parse_keys("SUPER+SHIFT+Return").unwrap();
        assert!(m.logo && m.shift && !m.ctrl);
        assert_eq!(t, Trigger::Key(Keysym::from(keysyms::KEY_Return)));
        let (_, t) = parse_keys("SUPER+q").unwrap();
        assert_eq!(t, Trigger::Key(Keysym::from(keysyms::KEY_q)));
        assert_eq!(parse_keys("SUPER+Q").unwrap().1, t);
        assert!(matches!(
            parse_keys("SUPER+Super_L:tap").unwrap().1,
            Trigger::Tap(_)
        ));
        assert!(parse_keys("HYPER+x").is_err());
        assert!(parse_keys("SUPER+NotAKey").is_err());
        assert_eq!(
            Action::parse("movetoworkspace 3 follow").unwrap(),
            Action::MoveToWorkspace {
                target: WorkspaceTarget::Id(3),
                follow: true
            }
        );
        assert_eq!(
            Action::parse("shell toggle launcher").unwrap(),
            Action::Shell(vec!["toggle".into(), "launcher".into()])
        );
        assert_eq!(
            Action::parse("workspace e+1").unwrap(),
            Action::Workspace(WorkspaceTarget::Next)
        );
        assert!(Action::parse("dance").is_err());
        assert!(Action::parse("exec").is_err());
    }

    #[test]
    fn defaults_cover_the_hyprland_set() {
        let d = defaults();
        let find = |keys: &str| {
            let (mods, trigger) = parse_keys(keys).unwrap();
            d.iter()
                .find(|b| b.mods == mods && b.trigger == trigger)
                .map(|b| b.action.clone())
        };
        assert_eq!(find("SUPER+Q"), Some(Action::Close));
        assert_eq!(
            find("SUPER+0"),
            Some(Action::Workspace(WorkspaceTarget::Id(10)))
        );
        assert_eq!(find("CTRL+ALT+F2"), Some(Action::VtSwitch(2)));
        assert!(d.iter().any(|b| b.locked && b.repeat));
        // No two built-ins share keys.
        for (i, a) in d.iter().enumerate() {
            for b in &d[i + 1..] {
                assert!(
                    !(a.mods == b.mods && a.trigger == b.trigger),
                    "{a:?} / {b:?}"
                );
            }
        }
    }

    #[test]
    fn user_binds_override_and_remove() {
        let (b, errors) = with_user(&[
            settings::Bind {
                keys: "SUPER+SHIFT+Return".into(),
                action: "exec kitty".into(),
            },
            settings::Bind {
                keys: "SUPER+Q".into(),
                action: "none".into(),
            },
            settings::Bind {
                keys: "SUPER+Nope".into(),
                action: "close".into(),
            },
        ]);
        assert_eq!(errors.len(), 1);
        let logo = Mods {
            logo: true,
            ..Default::default()
        };
        let shift = Mods {
            logo: true,
            shift: true,
            ..Default::default()
        };
        assert!(lookup(&b, logo, &[Keysym::from(keysyms::KEY_q)]).is_none());
        assert_eq!(
            lookup(&b, shift, &[Keysym::from(keysyms::KEY_Return)])
                .unwrap()
                .action,
            Action::Exec("kitty".into())
        );
        // Matching is on the lower-case symbol.
        assert_eq!(
            lookup(&b, logo, &[Keysym::from(keysyms::KEY_F)])
                .unwrap()
                .action,
            Action::ToggleMaximize
        );
        assert!(lookup_tap(&b, Keysym::from(keysyms::KEY_Super_L)).is_some());
    }
}
