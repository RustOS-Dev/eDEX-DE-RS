//! Services: RustOS's service manager (`svc`) for the system, and the session programs
//! edex-comp supervises (D-Bus, PipeWire, the shell…) for the user.

use anyhow::{anyhow, bail, Result};
use serde::Deserialize;

use crate::runner::CommandRunner;

#[derive(Clone, Debug, Default, PartialEq)]
pub struct UnitInfo {
    pub name: String,
    pub description: String,
    /// active | inactive | failed | activating
    pub active: String,
    /// running | dead | failed | starting
    pub sub: String,
    /// enabled | disabled | session
    pub enabled: String,
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct ServicesState {
    pub user: bool,
    pub units: Vec<UnitInfo>,
}

/// One entry of `svc list --json`.
#[derive(Debug, Deserialize)]
struct SvcEntry {
    name: String,
    #[serde(default)]
    description: String,
    enabled: bool,
    state: String,
}

pub fn parse_svc_list(json: &str) -> Result<Vec<UnitInfo>> {
    let entries: Vec<SvcEntry> = serde_json::from_str(json)?;
    let mut units: Vec<UnitInfo> = entries
        .into_iter()
        .map(|e| UnitInfo {
            active: match e.state.as_str() {
                "running" => "active",
                "starting" => "activating",
                "failed" => "failed",
                _ => "inactive",
            }
            .into(),
            sub: match e.state.as_str() {
                "stopped" => "dead".into(),
                other => other.to_string(),
            },
            enabled: if e.enabled { "enabled" } else { "disabled" }.into(),
            name: e.name,
            description: e.description,
        })
        .collect();
    units.sort_by(|a, b| a.name.cmp(&b.name));
    Ok(units)
}

pub fn query(r: &dyn CommandRunner, user: bool) -> ServicesState {
    let units = if user {
        session_programs()
    } else {
        r.run_ok("svc", &["list", "--json"])
            .ok()
            .and_then(|out| parse_svc_list(&out).ok())
            .unwrap_or_default()
    };
    ServicesState { user, units }
}

fn session_programs() -> Vec<UnitInfo> {
    let Some(socket) = comp::CompSocket::from_env() else {
        return Vec::new();
    };
    let mut units: Vec<UnitInfo> = socket
        .services()
        .unwrap_or_default()
        .into_iter()
        .map(|(name, running)| UnitInfo {
            description: "session program started by edex-comp".into(),
            active: if running { "active" } else { "inactive" }.into(),
            sub: if running { "running" } else { "dead" }.into(),
            enabled: "session".into(),
            name,
        })
        .collect();
    units.sort_by(|a, b| a.name.cmp(&b.name));
    units
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
    if user {
        let socket =
            comp::CompSocket::from_env().ok_or_else(|| anyhow!("edex-comp is not running"))?;
        return match action {
            UnitAction::Restart => socket.restart_service(unit),
            _ => bail!("session programs always run; they can only be restarted"),
        };
    }
    let verb = match action {
        UnitAction::Start => "start",
        UnitAction::Stop => "stop",
        UnitAction::Restart => "restart",
        UnitAction::Enable => "enable",
        UnitAction::Disable => "disable",
    };
    r.run_ok("svc", &[verb, unit]).map(|_| ())
}

pub fn is_active(r: &dyn CommandRunner, unit: &str) -> bool {
    r.run_ok("svc", &["status", unit, "--json"])
        .ok()
        .and_then(|out| serde_json::from_str::<SvcEntry>(&out).ok())
        .is_some_and(|e| e.state == "running")
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::runner::FakeRunner;

    const LIST: &str = r#"[
        {"name":"tor","description":"Tor anonymity daemon","enabled":false,"state":"stopped","pid":null,"tty":null},
        {"name":"rustos-nmd","description":"NetworkManager D-Bus service","enabled":true,"state":"running","pid":120,"tty":null},
        {"name":"edex","description":"eDEX desktop","enabled":true,"state":"failed","pid":null,"tty":"tty1"}
    ]"#;

    #[test]
    fn parses_svc() {
        let r = FakeRunner::default().with("svc list --json", LIST);
        let s = query(&r, false);
        assert_eq!(s.units.len(), 3);
        assert_eq!(s.units[0].name, "edex");
        assert_eq!(s.units[0].active, "failed");
        let nmd = s.units.iter().find(|u| u.name == "rustos-nmd").unwrap();
        assert_eq!(
            (nmd.active.as_str(), nmd.sub.as_str()),
            ("active", "running")
        );
        assert_eq!(nmd.enabled, "enabled");
        let tor = s.units.iter().find(|u| u.name == "tor").unwrap();
        assert_eq!(
            (tor.active.as_str(), tor.sub.as_str()),
            ("inactive", "dead")
        );
        let r = FakeRunner::default().with(
            "svc status rustos-nmd --json",
            r#"{"name":"rustos-nmd","enabled":true,"state":"running","pid":120,"tty":null}"#,
        );
        assert!(is_active(&r, "rustos-nmd"));
        assert!(!is_active(&r, "tor"));
        act(&r, false, "tor", UnitAction::Enable).unwrap_or(());
        assert!(r.calls().contains(&"svc enable tor".to_string()));
    }
}
