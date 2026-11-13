//! Starting featherplay when the user logs in: an XDG autostart entry,
//! `$XDG_CONFIG_HOME/autostart/dev.candygoogy.featherplay.desktop` (`~/.config` without
//! it), whose `Exec` is the AppImage (`$APPIMAGE`) or the binary, and `--autostart` (`FLAG`).
//!
//! A desktop's own startup settings turn an entry off without deleting it, with `Hidden=true`
//! or `X-GNOME-Autostart-enabled=false`. featherplay reads that as off
//! (`State::TurnedOffBySystem`) and doesn't turn it back on by itself; the user turning the
//! setting on in featherplay writes the entry afresh, as that's their latest word.

use std::fs;
use std::io;
use std::path::{Path, PathBuf};

pub use crate::common::autostart::*;

/// The entry, in an autostart folder.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Autostart {
    file: PathBuf,
}

impl Autostart {
    /// The current user's entry, in `$XDG_CONFIG_HOME/autostart` or `~/.config/autostart`.
    pub fn for_user() -> Result<Autostart, String> {
        let config = std::env::var_os("XDG_CONFIG_HOME")
            .map(PathBuf::from)
            .filter(|p| p.is_absolute())
            .or_else(|| std::env::var_os("HOME").map(|h| PathBuf::from(h).join(".config")))
            .ok_or("neither XDG_CONFIG_HOME nor HOME is set")?;
        Ok(Autostart::at(&config.join("autostart")))
    }

    /// The entry in the autostart folder `dir`.
    fn at(dir: &Path) -> Autostart {
        Autostart {
            file: dir.join(format!("{}.desktop", crate::APP_ID)),
        }
    }

    /// An entry of a test's own, named `name`, in a folder under the system's temp folder:
    /// never the one the desktop runs. `remove_for_tests` deletes it.
    #[doc(hidden)]
    pub fn for_tests(name: &str) -> Autostart {
        let dir =
            std::env::temp_dir().join(format!("featherplay-test-{name}-{}", std::process::id()));
        Autostart::at(&dir)
    }

    /// Turns the entry off as a desktop's startup settings do (`Hidden=true`), for tests.
    #[doc(hidden)]
    pub fn turn_off_for_tests(&self) -> Result<(), String> {
        let text = fs::read_to_string(&self.file).map_err(|e| e.to_string())?;
        fs::write(&self.file, format!("{text}Hidden=true\n")).map_err(|e| e.to_string())
    }

    /// Deletes a `for_tests` entry's folder; any other entry is left alone.
    #[doc(hidden)]
    pub fn remove_for_tests(&self) {
        let Some(dir) = self.file.parent() else {
            return;
        };
        let ours = dir.starts_with(std::env::temp_dir())
            && dir
                .file_name()
                .is_some_and(|n| n.to_string_lossy().starts_with("featherplay-test-"));
        if ours {
            let _ = fs::remove_dir_all(dir);
        }
    }

    /// Whether featherplay starts at login, as the desktop will see it.
    pub fn state(&self) -> State {
        match fs::read_to_string(&self.file) {
            Err(_) => State::Off,
            Ok(text) if turned_off(&text) => State::TurnedOffBySystem,
            Ok(_) => State::On,
        }
    }

    /// The entry's `Exec` line's value, if there's an entry.
    pub fn command(&self) -> Option<String> {
        let text = fs::read_to_string(&self.file).ok()?;
        key(&text, "Exec").map(str::to_string)
    }

    /// Starts `exe` at login from now on.
    pub fn enable(&self, exe: &Path) -> Result<(), String> {
        let write = || -> io::Result<()> {
            if let Some(dir) = self.file.parent() {
                fs::create_dir_all(dir)?;
            }
            fs::write(&self.file, entry(exe))
        };
        write().map_err(|e| format!("{}: {e}", self.file.display()))
    }

    /// Takes the entry away.
    pub fn disable(&self) -> Result<(), String> {
        match fs::remove_file(&self.file) {
            Ok(()) => Ok(()),
            Err(e) if e.kind() == io::ErrorKind::NotFound => Ok(()),
            Err(e) => Err(format!("{}: {e}", self.file.display())),
        }
    }

    /// Points an entry that's there at `exe`, if its `Exec` names another path (the AppImage
    /// moved). One the desktop turned off stays off: only `Exec` changes. Returns whether it
    /// changed anything.
    pub fn refresh(&self, exe: &Path) -> Result<bool, String> {
        let Ok(text) = fs::read_to_string(&self.file) else {
            return Ok(false);
        };
        let wanted = exec(exe);
        if key(&text, "Exec") == Some(wanted.as_str()) {
            return Ok(false);
        }
        let mut lines: Vec<String> = text
            .lines()
            .map(|l| {
                if l.split_once('=').is_some_and(|(k, _)| k.trim() == "Exec") {
                    format!("Exec={wanted}")
                } else {
                    l.to_string()
                }
            })
            .collect();
        if key(&text, "Exec").is_none() {
            lines.push(format!("Exec={wanted}"));
        }
        let mut out = lines.join("\n");
        out.push('\n');
        fs::write(&self.file, out).map_err(|e| format!("{}: {e}", self.file.display()))?;
        Ok(true)
    }
}

/// The program featherplay's entry should start: the AppImage this copy runs from, or else
/// this binary.
pub fn launch_path() -> Result<PathBuf, String> {
    if let Some(image) = std::env::var_os("APPIMAGE")
        .map(PathBuf::from)
        .filter(|p| p.is_absolute() && p.is_file())
    {
        return Ok(image);
    }
    std::env::current_exe().map_err(|e| format!("the binary's path: {e}"))
}

/// The entry for `exe`.
fn entry(exe: &Path) -> String {
    format!(
        "[Desktop Entry]\n\
         Type=Application\n\
         Name=featherplay\n\
         Comment=Starts featherplay when you log in\n\
         Exec={}\n\
         Icon={}\n\
         Terminal=false\n\
         X-GNOME-Autostart-enabled=true\n",
        exec(exe),
        crate::APP_ID
    )
}

/// The `Exec` value for `exe`: its path quoted as the Desktop Entry spec says (inside double
/// quotes, `"`, `` ` ``, `$` and `\` escaped by a backslash, `%` doubled), the backslashes then
/// escaped once more as every string value's are, and the flag.
fn exec(exe: &Path) -> String {
    let mut quoted = String::from("\"");
    for c in exe.to_string_lossy().chars() {
        match c {
            '"' | '`' | '$' => {
                quoted.push_str("\\\\");
                quoted.push(c);
            }
            '\\' => quoted.push_str("\\\\\\\\"),
            '%' => quoted.push_str("%%"),
            _ => quoted.push(c),
        }
    }
    quoted.push('"');
    format!("{quoted} {FLAG}")
}

/// A key's value in the entry's `[Desktop Entry]` group.
fn key<'a>(text: &'a str, name: &str) -> Option<&'a str> {
    let mut in_group = false;
    for line in text.lines() {
        let line = line.trim();
        if line.starts_with('[') {
            in_group = line == "[Desktop Entry]";
            continue;
        }
        if !in_group {
            continue;
        }
        if let Some((k, v)) = line.split_once('=')
            && k.trim() == name
        {
            return Some(v.trim());
        }
    }
    None
}

/// Whether the desktop's startup settings turned the entry off.
fn turned_off(text: &str) -> bool {
    key(text, "Hidden").is_some_and(|v| v.eq_ignore_ascii_case("true"))
        || key(text, "X-GNOME-Autostart-enabled").is_some_and(|v| v.eq_ignore_ascii_case("false"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn exec_quotes_the_path() {
        assert_eq!(
            exec(Path::new("/home/u/Apps/featherplay.AppImage")),
            "\"/home/u/Apps/featherplay.AppImage\" --autostart"
        );
        assert_eq!(
            exec(Path::new("/home/u/My $Apps/100%/fp")),
            "\"/home/u/My \\\\$Apps/100%%/fp\" --autostart"
        );
    }

    #[test]
    fn on_off_refresh_and_the_desktop_turning_it_off() {
        let a = Autostart::for_tests("autostart");
        a.remove_for_tests();
        let exe = Path::new("/home/u/featherplay.AppImage");
        assert_eq!(a.state(), State::Off);
        assert!(!a.refresh(exe).unwrap(), "no entry: nothing is made");
        a.enable(exe).unwrap();
        assert_eq!(a.state(), State::On);
        assert_eq!(a.command(), Some(exec(exe)));
        assert!(!a.refresh(exe).unwrap());
        // The desktop's settings turn it off: off, and a move keeps it off.
        a.turn_off_for_tests().unwrap();
        assert_eq!(a.state(), State::TurnedOffBySystem);
        let moved = Path::new("/opt/featherplay.AppImage");
        assert!(a.refresh(moved).unwrap());
        assert_eq!(a.command(), Some(exec(moved)));
        assert_eq!(a.state(), State::TurnedOffBySystem);
        // Turned on again in featherplay.
        a.enable(moved).unwrap();
        assert_eq!(a.state(), State::On);
        a.disable().unwrap();
        assert_eq!(a.state(), State::Off);
        a.disable().unwrap();
        a.remove_for_tests();
    }
}
