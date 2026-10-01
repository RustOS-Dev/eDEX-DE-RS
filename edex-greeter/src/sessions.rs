//! Sessions edex-comp can start. On RustOS that is the eDEX desktop; session files in
//! /usr/share/wayland-sessions that name edex-de are listed too (e.g. a safe-mode variant).

use std::path::PathBuf;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Session {
    pub id: String,
    pub name: String,
    pub exec: Vec<String>,
    pub x11: bool,
}

fn parse_desktop(id: &str, text: &str, x11: bool) -> Option<Session> {
    let mut name = None;
    let mut exec = None;
    let mut in_entry = false;
    for line in text.lines() {
        let l = line.trim();
        if l.starts_with('[') {
            in_entry = l == "[Desktop Entry]";
            continue;
        }
        if !in_entry {
            continue;
        }
        if let Some(v) = l.strip_prefix("Name=") {
            name.get_or_insert(v.trim().to_string());
        } else if let Some(v) = l.strip_prefix("Exec=") {
            exec = Some(v.trim().to_string());
        } else if l == "Hidden=true" || l == "NoDisplay=true" {
            return None;
        }
    }
    let exec = exec?;
    let mut cmd: Vec<String> = exec.split_whitespace().map(|s| s.to_string()).collect();
    if cmd.is_empty() {
        return None;
    }
    if x11 {
        cmd.insert(0, "startx".into());
    }
    Some(Session {
        id: id.to_string(),
        name: name.unwrap_or_else(|| id.to_string()),
        exec: cmd,
        x11,
    })
}

pub fn scan(dirs: &[(PathBuf, bool)]) -> Vec<Session> {
    let mut out = Vec::new();
    for (dir, x11) in dirs {
        let Ok(rd) = std::fs::read_dir(dir) else {
            continue;
        };
        let mut files: Vec<PathBuf> = rd
            .flatten()
            .map(|e| e.path())
            .filter(|p| p.extension().is_some_and(|e| e == "desktop"))
            .collect();
        files.sort();
        for f in files {
            let id = f
                .file_stem()
                .and_then(|s| s.to_str())
                .unwrap_or("")
                .to_string();
            if out.iter().any(|s: &Session| s.id == id) {
                continue;
            }
            if let Ok(text) = std::fs::read_to_string(&f) {
                if let Some(s) = parse_desktop(&id, &text, *x11) {
                    out.push(s);
                }
            }
        }
    }
    out
}

pub fn default_dirs() -> Vec<(PathBuf, bool)> {
    vec![
        (PathBuf::from("/usr/share/wayland-sessions"), false),
        (PathBuf::from("/usr/local/share/wayland-sessions"), false),
        (PathBuf::from("/usr/share/xsessions"), true),
    ]
}

/// The sessions offered on the login screen: eDEX-DE first, then other session files whose
/// command is an eDEX session.
pub fn edex_sessions() -> Vec<Session> {
    let mut out = vec![Session {
        id: "edex-de".into(),
        name: "eDEX-DE".into(),
        exec: vec!["edex-de".into(), "run".into()],
        x11: false,
    }];
    for s in scan(&default_dirs()) {
        if s.id != "edex-de" && s.exec.first().is_some_and(|c| c.starts_with("edex")) {
            out.push(s);
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_session_files() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(
            dir.path().join("edex-de.desktop"),
            "[Desktop Entry]\nName=eDEX-DE\nExec=edex-session\nType=Application\n",
        )
        .unwrap();
        std::fs::write(
            dir.path().join("hidden.desktop"),
            "[Desktop Entry]\nName=Hidden\nExec=x\nHidden=true\n",
        )
        .unwrap();
        let s = scan(&[(dir.path().to_path_buf(), false)]);
        assert_eq!(s.len(), 1);
        assert_eq!(s[0].exec, vec!["edex-session"]);
        assert_eq!(s[0].name, "eDEX-DE");
    }
}
