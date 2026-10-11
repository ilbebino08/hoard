//! What an older Hoard left on this machine, found and taken off.
//!
//! Up to 1.2.x the core shipped twice: in the tarball the terminal installer and
//! Hoard Setup used, and inside every app package as a sidecar (`/usr/bin` from a
//! `.deb`, beside the app in the NSIS folder, inside `Hoard.app`). A machine that
//! went down both roads kept both pairs, nothing ever removed either, and the
//! service ran whichever one it had been pointed at first. This is the list of
//! those places and the way out of each, so that an install or an update leaves
//! exactly one Hoard behind.
//!
//! What it never touches: saves, settings, the local database. Same rule as
//! [`super::remove`].

#[cfg(unix)]
use std::path::Path;
use std::path::PathBuf;

use anyhow::{Context, Result};

#[cfg(unix)]
use super::layout;
use super::{remove, Delivery};

/// One piece of an older install.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Leftover {
    /// The app as a `.deb` or `.rpm`. It also put `hoard` and `hoardd` in
    /// `/usr/bin`, and they go with it.
    Package { delivery: Delivery, app: PathBuf },
    /// The per-user NSIS install: the app, both sidecars and the uninstaller,
    /// in `%LOCALAPPDATA%\Hoard`.
    Nsis { dir: PathBuf },
    /// The per-machine MSI, in Program Files.
    Msi { product_code: String },
    /// `hoard` and `hoardd` somewhere other than [`layout::core_dir`].
    LooseCore { dir: PathBuf },
    /// The copy of an AppImage's engine that login start ran before the core
    /// had a place of its own.
    StagedDaemon { dir: PathBuf },
    /// A link to the terminal inside an app bundle, from when the app put
    /// `hoard` on the PATH by itself.
    CliLink { path: PathBuf },
}

impl Leftover {
    /// Taking it off needs an administrator (polkit, UAC). Without somebody to
    /// ask, it stays where it is and the install says so.
    pub fn needs_admin(&self) -> bool {
        matches!(self, Leftover::Package { .. } | Leftover::Msi { .. })
    }

    /// It has to go before the new files are written, not after.
    ///
    /// The NSIS uninstaller kills every `hoardd.exe` by name, and the ones built
    /// with the 05-10 hook (`47e12e4c`) also delete `%LOCALAPPDATA%\hoard\bin`
    /// and the login task: run after the new install, it would take the new
    /// install with it.
    pub fn in_the_way(&self) -> bool {
        matches!(self, Leftover::Nsis { .. })
    }

    /// For logs and for the installer's last screen.
    pub fn describe(&self) -> String {
        match self {
            Leftover::Package { delivery, app } => {
                format!("the {} package ({})", delivery.as_str(), app.display())
            }
            Leftover::Nsis { dir } => format!("the previous installer's copy in {}", dir.display()),
            Leftover::Msi { product_code } => format!("the MSI install ({product_code})"),
            Leftover::LooseCore { dir } => {
                format!("an older hoard and hoardd in {}", dir.display())
            }
            Leftover::StagedDaemon { dir } => format!("an old copy of hoardd in {}", dir.display()),
            Leftover::CliLink { path } => format!("a stale link at {}", path.display()),
        }
    }
}

/// Everything an older install left here. Read-only: it runs nothing that
/// changes the machine, only `--version`, `dpkg -S` and the like.
pub async fn scan() -> Vec<Leftover> {
    let mut out = Vec::new();
    // A Flatpak sees the host's home through `--filesystem=host`, and every
    // path below would come back as something to remove. Nothing in here is
    // ours to touch from inside the sandbox.
    if super::running_under_flatpak() {
        return out;
    }
    platform(&mut out).await;
    out
}

/// Takes one leftover off. `interactive` allows the password prompt.
pub async fn remove(item: &Leftover, interactive: bool) -> Result<()> {
    match item {
        Leftover::Package { delivery, app } => {
            remove::desktop(*delivery, Some(app.as_path()), !interactive).await?;
        }
        Leftover::Nsis { dir } => {
            remove::nsis(dir).await?;
            // The app put its own folder on the user's PATH to make `hoard`
            // typable; the core has a folder of its own now.
            remove::shell_path_line(dir);
        }
        Leftover::Msi { product_code } => remove::msi(product_code, interactive).await?,
        Leftover::LooseCore { dir } => {
            remove::core(dir)?;
        }
        Leftover::StagedDaemon { dir } => {
            // `hoard` too: an updater up to 1.2.1 relieved the core wherever the
            // service ran from, and this copy was it.
            for name in ["hoardd", "hoard", "hoardd.version", "hoardd.staging"] {
                let path = dir.join(name);
                if path.exists() {
                    std::fs::remove_file(&path)
                        .with_context(|| format!("deleting {}", path.display()))?;
                }
            }
            let _ = std::fs::remove_dir(dir);
        }
        Leftover::CliLink { path } => {
            std::fs::remove_file(path).with_context(|| format!("deleting {}", path.display()))?;
        }
    }
    Ok(())
}

/// Does this binary answer `--version` as one of ours? Asked rather than
/// assumed from the name: `/usr/local/bin/hoard` is a short enough name for
/// somebody else's tool to have taken it.
#[cfg(unix)]
async fn is_ours(path: &Path) -> bool {
    let Some(name) = path.file_name().and_then(|n| n.to_str()) else {
        return false;
    };
    let run = tokio::process::Command::new(path)
        .arg("--version")
        .stdin(std::process::Stdio::null())
        .output();
    let Ok(Ok(out)) = tokio::time::timeout(std::time::Duration::from_secs(5), run).await else {
        return false;
    };
    let text = String::from_utf8_lossy(&out.stdout);
    text.trim().starts_with(&format!("{name} "))
}

/// A `hoard`/`hoardd` pair in `dir`, when it is ours and not the core.
#[cfg(unix)]
async fn loose_core(dir: &Path, out: &mut Vec<Leftover>) {
    let Ok(core) = layout::core_dir() else {
        return;
    };
    let same = |a: &Path, b: &Path| {
        std::fs::canonicalize(a).unwrap_or_else(|_| a.to_path_buf())
            == std::fs::canonicalize(b).unwrap_or_else(|_| b.to_path_buf())
    };
    if same(dir, &core) {
        return;
    }
    let mut ours = false;
    for name in ["hoardd", "hoard"] {
        let path = dir.join(name);
        // A link is the app's old doing ([`Leftover::CliLink`]), not a copy.
        if path.is_symlink() {
            continue;
        }
        if path.is_file() && is_ours(&path).await {
            ours = true;
        }
    }
    if ours {
        out.push(Leftover::LooseCore {
            dir: dir.to_path_buf(),
        });
    }
}

/// `hoard` as a link into an app bundle or an AppImage mount.
#[cfg(unix)]
fn cli_link(path: &Path, out: &mut Vec<Leftover>) {
    if !path.is_symlink() {
        return;
    }
    let Ok(target) = std::fs::read_link(path) else {
        return;
    };
    let target = target.to_string_lossy();
    if target.contains("/Hoard.app/") || target.contains("/.mount_") {
        out.push(Leftover::CliLink {
            path: path.to_path_buf(),
        });
    }
}

#[cfg(target_os = "linux")]
async fn platform(out: &mut Vec<Leftover>) {
    let app = Path::new("/usr/bin/hoard-desktop");
    if app.is_file() {
        // Owned by neither dpkg nor rpm means somebody else's package (an AUR
        // build, a Nix profile): theirs to update and theirs to remove.
        if let Some(delivery) = remove::package_manager_of(app).await {
            out.push(Leftover::Package {
                delivery,
                app: app.to_path_buf(),
            });
        }
    }
    if let Ok(home) = layout::home() {
        let local_bin = home.join(".local").join("bin");
        loose_core(&local_bin, out).await;
        cli_link(&local_bin.join("hoard"), out);
    }
    // Where `HOARD_INSTALL_DIR=/usr/local/bin` left it. Only if it can be
    // removed without root: a root-owned copy there is somebody's deliberate
    // choice, not debris.
    let usr_local = Path::new("/usr/local/bin");
    if writable(usr_local) {
        loose_core(usr_local, out).await;
    }
    if let Some(dir) = staged_daemon_dir() {
        if dir.join("hoardd").is_file() {
            out.push(Leftover::StagedDaemon { dir });
        }
    }
}

/// `autostart`'s stable copy of an AppImage's `hoardd`, from before the core
/// was always installed next to the app.
#[cfg(target_os = "linux")]
pub(super) fn staged_daemon_dir() -> Option<PathBuf> {
    let base = std::env::var_os("XDG_DATA_HOME")
        .filter(|s| !s.is_empty())
        .map(PathBuf::from)
        .or_else(|| layout::home().ok().map(|h| h.join(".local").join("share")))?;
    Some(base.join("hoard").join("bin"))
}

#[cfg(target_os = "macos")]
async fn platform(out: &mut Vec<Leftover>) {
    if let Ok(home) = layout::home() {
        let local_bin = home.join(".local").join("bin");
        loose_core(&local_bin, out).await;
        cli_link(&local_bin.join("hoard"), out);
    }
    cli_link(Path::new("/usr/local/bin/hoard"), out);
}

#[cfg(windows)]
async fn platform(out: &mut Vec<Leftover>) {
    if let Some(local) = std::env::var_os("LOCALAPPDATA").map(PathBuf::from) {
        let dir = local.join("Hoard");
        if dir.join("uninstall.exe").is_file() {
            out.push(Leftover::Nsis { dir });
        }
    }
    for product_code in super::windows::msi_installs() {
        out.push(Leftover::Msi { product_code });
    }
}

#[cfg(not(any(target_os = "linux", target_os = "macos", windows)))]
async fn platform(_out: &mut Vec<Leftover>) {}

#[cfg(target_os = "linux")]
fn writable(dir: &Path) -> bool {
    let probe = dir.join(".hoard-write-probe");
    match std::fs::write(&probe, b"") {
        Ok(()) => {
            let _ = std::fs::remove_file(&probe);
            true
        }
        Err(_) => false,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_packages_and_msi_need_an_administrator() {
        let admin = [
            Leftover::Package {
                delivery: Delivery::Deb,
                app: "/usr/bin/hoard-desktop".into(),
            },
            Leftover::Msi {
                product_code: "{X}".into(),
            },
        ];
        let plain = [
            Leftover::Nsis { dir: "C:/x".into() },
            Leftover::LooseCore { dir: "/x".into() },
            Leftover::StagedDaemon { dir: "/x".into() },
            Leftover::CliLink { path: "/x".into() },
        ];
        assert!(admin.iter().all(Leftover::needs_admin));
        assert!(!plain.iter().any(Leftover::needs_admin));
    }

    /// The order the installer depends on: only what would undo the new
    /// install goes first.
    #[test]
    fn only_the_nsis_folder_has_to_go_before_the_new_files() {
        assert!(Leftover::Nsis { dir: "C:/x".into() }.in_the_way());
        assert!(!Leftover::Package {
            delivery: Delivery::Rpm,
            app: "/usr/bin/hoard-desktop".into(),
        }
        .in_the_way());
        assert!(!Leftover::LooseCore { dir: "/x".into() }.in_the_way());
    }

    #[cfg(unix)]
    #[test]
    fn a_link_into_a_bundle_is_a_leftover_and_a_real_file_is_not() {
        let dir = tempfile::tempdir().unwrap();
        let link = dir.path().join("hoard");
        std::os::unix::fs::symlink("/Applications/Hoard.app/Contents/MacOS/hoard", &link).unwrap();
        let mut out = Vec::new();
        cli_link(&link, &mut out);
        assert_eq!(out, vec![Leftover::CliLink { path: link.clone() }]);

        std::fs::remove_file(&link).unwrap();
        std::fs::write(&link, b"#!/bin/sh\n").unwrap();
        let mut out = Vec::new();
        cli_link(&link, &mut out);
        assert!(out.is_empty());
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn somebody_elses_hoard_is_not_ours() {
        use std::os::unix::fs::PermissionsExt;
        let dir = tempfile::tempdir().unwrap();
        let fake = dir.path().join("hoard");
        std::fs::write(&fake, "#!/bin/sh\necho 'hoard-of-gold 3.0'\n").unwrap();
        std::fs::set_permissions(&fake, std::fs::Permissions::from_mode(0o755)).unwrap();
        assert!(!is_ours(&fake).await);

        std::fs::write(&fake, "#!/bin/sh\necho 'hoard 1.2.0'\n").unwrap();
        assert!(is_ours(&fake).await);
    }
}
