use std::env;
use std::path::{Path, PathBuf};

/// Where featherplay keeps its files. Nothing is created until something is saved.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Paths {
    pub config: PathBuf,
    pub cache: PathBuf,
}

impl Paths {
    /// `FEATHERPLAY_HOME` gives `{home}/config` and `{home}/cache`. Otherwise
    /// `%APPDATA%/featherplay` and `%LOCALAPPDATA%/featherplay`.
    pub fn resolve() -> Paths {
        if let Some(home) = env::var_os("FEATHERPLAY_HOME").filter(|v| !v.is_empty()) {
            return Paths::from_home(Path::new(&home));
        }
        let base = |var: &str| {
            env::var_os(var)
                .map(PathBuf::from)
                .unwrap_or_default()
                .join("featherplay")
        };
        Paths {
            config: base("APPDATA"),
            cache: base("LOCALAPPDATA"),
        }
    }

    /// The layout `resolve` uses for `FEATHERPLAY_HOME`, without reading the environment.
    pub fn from_home(home: &Path) -> Paths {
        Paths {
            config: home.join("config"),
            cache: home.join("cache"),
        }
    }
}
