//! XDG paths used by the shell.

use std::path::PathBuf;

fn home() -> PathBuf {
    std::env::var("HOME")
        .map(PathBuf::from)
        .unwrap_or_else(|_| PathBuf::from("/tmp"))
}

pub fn config_dir() -> PathBuf {
    std::env::var("XDG_CONFIG_HOME")
        .map(PathBuf::from)
        .unwrap_or_else(|_| home().join(".config"))
        .join("edex-de")
}

pub fn config_path() -> PathBuf {
    config_dir().join("config.toml")
}

pub fn state_dir() -> PathBuf {
    std::env::var("XDG_STATE_HOME")
        .map(PathBuf::from)
        .unwrap_or_else(|_| home().join(".local/state"))
        .join("edex-de")
}

pub fn user_theme_dir() -> PathBuf {
    config_dir().join("themes")
}

/// The first of `candidates` that exists, else the last one.
pub fn first_existing<P: Into<PathBuf>>(candidates: impl IntoIterator<Item = P>) -> PathBuf {
    let mut last = PathBuf::new();
    for c in candidates {
        let c = c.into();
        if c.exists() {
            return c;
        }
        last = c;
    }
    last
}

/// System data directory: `$EDEX_SHARE_DIR` (development), `/usr/local/share/edex-de` (the
/// RustOS port installs under `/usr/local`) or `/usr/share/edex-de`, the first that exists.
pub fn system_share_dir() -> PathBuf {
    let env = std::env::var_os("EDEX_SHARE_DIR").map(PathBuf::from);
    first_existing(
        env.into_iter()
            .chain(["/usr/local/share/edex-de", "/usr/share/edex-de"].map(PathBuf::from)),
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn first_existing_falls_back_to_the_last() {
        let dir = std::env::temp_dir();
        assert_eq!(
            first_existing([dir.join("edex-no-such-dir"), dir.clone()]),
            dir
        );
        assert_eq!(
            first_existing(["/edex-no-such-a", "/edex-no-such-b"]),
            PathBuf::from("/edex-no-such-b")
        );
    }
}
