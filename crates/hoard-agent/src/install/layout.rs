//! Where each piece of Hoard lives on a machine. One answer, asked by everyone
//! who installs, updates, repairs or removes it.
//!
//! Before this there was an answer per front door. The terminal script put the
//! core in `~/.local/bin`, a `.deb` put another one in `/usr/bin`, the Windows
//! installer kept its own pair beside the app, and which `hoardd` ran depended on
//! who happened to start it. Two cores on one machine is how a 1.2.1 app ended up
//! talking to a 1.2.0 engine that no update was ever going to touch: the update
//! relieved the copy in the package and the service kept starting the other one.

use std::path::{Path, PathBuf};

use anyhow::{Context, Result};

/// Moves the core somewhere else. The terminal installer has honoured it since
/// its first version, so a machine that used it still has its core there.
pub const INSTALL_DIR_ENV: &str = "HOARD_INSTALL_DIR";

/// `hoard` and `hoardd`. Never anywhere else: the service, the PATH and every
/// update point here.
///
/// The variable only lives in the shell that ran the installer. The service,
/// the app and a repair started by either never see it, so the folder an
/// install chose is read back from the record it left: without that, the
/// service took a chosen folder for an install out of place, held every update
/// back and had the repair move the core to the default one.
pub fn core_dir() -> Result<PathBuf> {
    let overridden = std::env::var_os(INSTALL_DIR_ENV).filter(|d| !d.is_empty());
    if overridden.is_none() {
        if let Some(dir) = recorded_core_dir() {
            return Ok(dir);
        }
    }
    core_dir_with(overridden)
}

fn recorded_core_dir() -> Option<PathBuf> {
    let dir = super::Manifest::load().ok().flatten()?.core_dir?;
    is_chosen_core(&dir).then_some(dir)
}

/// Does this recorded folder look like a core somebody put there, rather than
/// where an older release kept its copy? Those carried the app beside the core
/// (`/usr/bin`, the NSIS folder, inside `Hoard.app`), or were the AppImage's
/// staging copy, and are exactly what a repair moves away from.
fn is_chosen_core(dir: &Path) -> bool {
    let has = |name: &str| {
        dir.join(format!("{name}{}", std::env::consts::EXE_SUFFIX))
            .is_file()
    };
    #[cfg(target_os = "linux")]
    if super::legacy::staged_daemon_dir().is_some_and(|staged| staged == dir) {
        return false;
    }
    has("hoard") && has("hoardd") && !has("hoard-desktop") && writable(dir)
}

/// Asked without writing anything: this runs on every client connection.
#[cfg(unix)]
fn writable(dir: &Path) -> bool {
    use std::os::unix::ffi::OsStrExt;
    let Ok(path) = std::ffi::CString::new(dir.as_os_str().as_bytes()) else {
        return false;
    };
    // SAFETY: a NUL-terminated path and a flag; `access` reads nothing else.
    unsafe { libc::access(path.as_ptr(), libc::W_OK) == 0 }
}

#[cfg(not(unix))]
fn writable(_dir: &Path) -> bool {
    true
}

fn core_dir_with(overridden: Option<std::ffi::OsString>) -> Result<PathBuf> {
    if let Some(dir) = overridden.filter(|d| !d.is_empty()) {
        return Ok(PathBuf::from(dir));
    }
    #[cfg(windows)]
    {
        Ok(local_app_data()?.join("hoard").join("bin"))
    }
    #[cfg(not(windows))]
    {
        Ok(home()?.join(".local").join("bin"))
    }
}

/// One of the two core binaries, by bare name (`"hoard"`, `"hoardd"`).
pub fn core_binary(name: &str) -> Result<PathBuf> {
    Ok(core_dir()?.join(format!("{name}{}", std::env::consts::EXE_SUFFIX)))
}

/// Where the app goes on this system.
///
/// - Linux: the AppImage, as `~/.local/bin/hoard-desktop`. It is where every
///   AppImage install so far has put it, so an update finds it in place.
/// - Windows: `%LOCALAPPDATA%\hoard\app\hoard-desktop.exe`. A folder of its own
///   and not the one the old NSIS installer used (`%LOCALAPPDATA%\Hoard`, the
///   same folder on a case-insensitive disk): that installer's uninstaller
///   deletes `hoard-desktop.exe` from there, and it runs during the move.
/// - macOS: the bundle in `/Applications`, or in `~/Applications` for an
///   account that can't write the first one (`fetch::copy_app_out` falls back
///   there). Answering `/Applications` regardless made every such install look
///   out of place for good: updates held back and a repair a day.
pub fn app_path() -> Result<PathBuf> {
    #[cfg(target_os = "linux")]
    {
        Ok(home()?.join(".local").join("bin").join("hoard-desktop"))
    }
    #[cfg(windows)]
    {
        Ok(app_dir()?.join("hoard-desktop.exe"))
    }
    #[cfg(target_os = "macos")]
    {
        let system = PathBuf::from("/Applications/Hoard.app");
        if !system.exists() {
            let user = home()?.join("Applications").join("Hoard.app");
            if user.exists() {
                return Ok(user);
            }
        }
        Ok(system)
    }
    #[cfg(not(any(target_os = "linux", windows, target_os = "macos")))]
    {
        anyhow::bail!("no desktop app for this platform")
    }
}

/// The folder Hoard owns on Windows, with the core in `bin\` and the app in
/// `app\`. The user's data is NOT here: it is under `hoard\hoard\`, which
/// belongs to `CliConfig`, and nothing that removes Hoard touches it.
#[cfg(windows)]
pub fn root_dir() -> Result<PathBuf> {
    Ok(local_app_data()?.join("hoard"))
}

#[cfg(windows)]
fn app_dir() -> Result<PathBuf> {
    Ok(root_dir()?.join("app"))
}

/// The copy of Hoard Setup that "Installed apps" runs to uninstall.
#[cfg(windows)]
pub fn setup_copy() -> Result<PathBuf> {
    Ok(root_dir()?.join("HoardSetup.exe"))
}

#[cfg(windows)]
fn local_app_data() -> Result<PathBuf> {
    std::env::var_os("LOCALAPPDATA")
        .map(PathBuf::from)
        .context("no LOCALAPPDATA in the environment")
}

#[cfg_attr(windows, allow(dead_code))]
pub(crate) fn home() -> Result<PathBuf> {
    std::env::var_os("HOME")
        .or_else(|| std::env::var_os("USERPROFILE"))
        .filter(|h| !h.is_empty())
        .map(PathBuf::from)
        .context("no HOME in the environment")
}

/// Is `path` the core this machine is supposed to run? Compared after
/// resolving links, since `~/.local/bin` is a symlink on more systems than
/// anyone expects.
pub fn is_core(path: &std::path::Path) -> bool {
    let Ok(dir) = core_dir() else {
        return false;
    };
    let canonical =
        |p: &std::path::Path| std::fs::canonicalize(p).unwrap_or_else(|_| p.to_path_buf());
    path.parent()
        .is_some_and(|parent| canonical(parent) == canonical(&dir))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_install_dir_override_moves_the_core() {
        let dir = core_dir_with(Some("/opt/hoard-test/bin".into())).unwrap();
        assert_eq!(dir, PathBuf::from("/opt/hoard-test/bin"));
        // An empty value is the variable being unset, not "the current directory".
        assert_ne!(core_dir_with(Some("".into())).unwrap(), PathBuf::from(""));
    }

    /// A folder chosen with `HOARD_INSTALL_DIR` holds the core alone; one with
    /// the app beside it is an older package's, and stays something to move off.
    #[test]
    fn a_recorded_folder_counts_only_when_it_holds_the_core_alone() {
        let dir = tempfile::tempdir().unwrap();
        let exe = std::env::consts::EXE_SUFFIX;
        std::fs::write(dir.path().join(format!("hoard{exe}")), b"").unwrap();
        assert!(!is_chosen_core(dir.path()), "half a core is not a core");
        std::fs::write(dir.path().join(format!("hoardd{exe}")), b"").unwrap();
        assert!(is_chosen_core(dir.path()));
        std::fs::write(dir.path().join(format!("hoard-desktop{exe}")), b"").unwrap();
        assert!(!is_chosen_core(dir.path()));
    }

    #[cfg(windows)]
    #[test]
    fn the_app_is_not_where_the_old_installer_kept_it() {
        // `%LOCALAPPDATA%\Hoard\hoard-desktop.exe` is what the NSIS uninstaller
        // deletes, and the move to this layout runs that uninstaller.
        let app = app_path().unwrap();
        let old = local_app_data().unwrap().join("Hoard");
        assert_ne!(
            app.parent().unwrap().to_string_lossy().to_lowercase(),
            old.to_string_lossy().to_lowercase()
        );
    }
}
