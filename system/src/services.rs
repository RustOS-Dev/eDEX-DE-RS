//! systemd units (system and user managers) via `systemctl`.

use anyhow::Result;

use crate::runner::CommandRunner;

#[derive(Clone, Debug, Default, PartialEq)]
pub struct UnitInfo {
    pub name: String,
    pub description: String,
    pub active: String,
    pub sub: String,
    pub enabled: String,
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct ServicesState {
    pub user: bool,
    pub units: Vec<UnitInfo>,
}

fn scope_args(user: bool) -> Vec<&'static str> {
    if user {
        vec!["--user"]
    } else {
        vec![]
    }
}

pub fn query(r: &dyn CommandRunner, user: bool) -> ServicesState {
    let mut args = scope_args(user);
    args.extend([
        "list-units",
        "--type=service",
        "--all",
        "--no-legend",
        "--no-pager",
        "--plain",
    ]);
    let mut units = Vec::new();
    if let Ok(out) = r.run("systemctl", &args) {
        for line in out.stdout.lines() {
            let mut cols = line.split_whitespace();
            let (Some(name), Some(_load), Some(active), Some(sub)) =
                (cols.next(), cols.next(), cols.next(), cols.next())
            else {
                continue;
            };
            if name.contains('@') && !name.contains("getty") {
                continue;
            }
            units.push(UnitInfo {
                name: name.to_string(),
                description: cols.collect::<Vec<_>>().join(" "),
                active: active.into(),
                sub: sub.into(),
                enabled: String::new(),
            });
        }
    }
    let mut args = scope_args(user);
    args.extend([
        "list-unit-files",
        "--type=service",
        "--no-legend",
        "--no-pager",
        "--plain",
    ]);
    if let Ok(out) = r.run("systemctl", &args) {
        for line in out.stdout.lines() {
            let mut cols = line.split_whitespace();
            if let (Some(name), Some(state)) = (cols.next(), cols.next()) {
                if let Some(u) = units.iter_mut().find(|u| u.name == name) {
                    u.enabled = state.to_string();
                }
            }
        }
    }
    units.sort_by(|a, b| a.name.cmp(&b.name));
    ServicesState { user, units }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum UnitAction {
    Start,
    Stop,
    Restart,
    Enable,
    Disable,
}

pub fn act(r: &dyn CommandRunner, user: bool, unit: &str, action: UnitAction) -> Result<()> {
    let verb = match action {
        UnitAction::Start => "start",
        UnitAction::Stop => "stop",
        UnitAction::Restart => "restart",
        UnitAction::Enable => "enable",
        UnitAction::Disable => "disable",
    };
    if user {
        r.run_ok("systemctl", &["--user", verb, unit]).map(|_| ())
    } else {
        // System units need polkit; systemctl asks the agent via pkexec-style prompts.
        r.run_ok("systemctl", &[verb, unit]).map(|_| ())
    }
}

pub fn is_active(r: &dyn CommandRunner, unit: &str) -> bool {
    r.run("systemctl", &["is-active", "--quiet", unit])
        .map(|o| o.ok())
        .unwrap_or(false)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::runner::FakeRunner;

    #[test]
    fn parses_units() {
        let r = FakeRunner::default()
            .with("systemctl list-units --type=service --all --no-legend --no-pager --plain", "tor.service loaded inactive dead Anonymizing overlay network\nNetworkManager.service loaded active running Network Manager\n")
            .with("systemctl list-unit-files --type=service --no-legend --no-pager --plain", "tor.service disabled disabled\nNetworkManager.service enabled enabled\n");
        let s = query(&r, false);
        assert_eq!(s.units.len(), 2);
        let nm = s
            .units
            .iter()
            .find(|u| u.name == "NetworkManager.service")
            .unwrap();
        assert_eq!(nm.active, "active");
        assert_eq!(nm.enabled, "enabled");
        assert_eq!(nm.description, "Network Manager");
    }
}
