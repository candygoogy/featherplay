use std::env;
use std::path::{Path, PathBuf};

/// Where featherplay keeps its files. Nothing is created until something is saved.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Paths {
    pub config: PathBuf,
    pub cache: PathBuf,
    /// The logs. The cache folder on Windows and under `FEATHERPLAY_HOME`; on Linux
    /// `$XDG_STATE_HOME/featherplay`.
    pub state: PathBuf,
}

impl Paths {
    /// `FEATHERPLAY_HOME` gives `{home}/config` and `{home}/cache` (the logs in the cache
    /// too). Otherwise the OS's folders: on Windows `%APPDATA%/featherplay` and
    /// `%LOCALAPPDATA%/featherplay` (the logs there); on Linux the XDG base directories
    /// (`$XDG_CONFIG_HOME`, `$XDG_CACHE_HOME`, `$XDG_STATE_HOME`, by default `~/.config`,
    /// `~/.cache` and `~/.local/state`), each with a `featherplay` folder.
    pub fn resolve() -> Paths {
        if let Some(home) = env::var_os("FEATHERPLAY_HOME").filter(|v| !v.is_empty()) {
            return Paths::from_home(Path::new(&home));
        }
        Paths::system()
    }

    #[cfg(windows)]
    fn system() -> Paths {
        let base = |var: &str| {
            env::var_os(var)
                .map(PathBuf::from)
                .unwrap_or_default()
                .join("featherplay")
        };
        Paths {
            config: base("APPDATA"),
            cache: base("LOCALAPPDATA"),
            state: base("LOCALAPPDATA"),
        }
    }

    #[cfg(not(windows))]
    fn system() -> Paths {
        let home = env::var_os("HOME").map(PathBuf::from);
        let base = |var: &str, default: &str| {
            xdg_dir(
                env::var_os(var).map(PathBuf::from),
                home.as_deref(),
                default,
            )
            .join("featherplay")
        };
        Paths {
            config: base("XDG_CONFIG_HOME", ".config"),
            cache: base("XDG_CACHE_HOME", ".cache"),
            state: base("XDG_STATE_HOME", ".local/state"),
        }
    }

    /// The layout `resolve` uses for `FEATHERPLAY_HOME`, without reading the environment.
    pub fn from_home(home: &Path) -> Paths {
        Paths {
            config: home.join("config"),
            cache: home.join("cache"),
            state: home.join("cache"),
        }
    }
}

/// An XDG base directory: the variable's value when it's an absolute path (the spec says to
/// ignore a relative one), else `default` under the home folder.
#[cfg(not(windows))]
fn xdg_dir(value: Option<PathBuf>, home: Option<&Path>, default: &str) -> PathBuf {
    match value {
        Some(dir) if dir.is_absolute() => dir,
        _ => home.map(|h| h.join(default)).unwrap_or_default(),
    }
}

#[cfg(all(test, not(windows)))]
mod tests {
    use super::*;

    #[test]
    fn an_xdg_variable_wins_unless_it_is_relative_or_empty() {
        let home = Path::new("/home/ada");
        assert_eq!(
            xdg_dir(Some("/data/cfg".into()), Some(home), ".config"),
            PathBuf::from("/data/cfg")
        );
        assert_eq!(
            xdg_dir(Some("cfg".into()), Some(home), ".config"),
            PathBuf::from("/home/ada/.config")
        );
        assert_eq!(
            xdg_dir(Some("".into()), Some(home), ".local/state"),
            PathBuf::from("/home/ada/.local/state")
        );
        assert_eq!(
            xdg_dir(None, Some(home), ".cache"),
            PathBuf::from("/home/ada/.cache")
        );
    }
}
