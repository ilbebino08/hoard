//! What a Windows install needs besides the files: a Start menu entry, a line in
//! "Installed apps", the `hoard://` link and WebView2.
//!
//! The NSIS installer used to do all of this, and also everything wrong with
//! Windows installs: its uninstaller only knew its own folder, so removing Hoard
//! from Settings left the core and the login task behind and the next update put
//! the app back. Now the entry in Settings runs Hoard Setup, which removes what
//! [`super::remove`] knows about, and that is everything.

use std::path::Path;

use anyhow::{Context, Result};
use winreg::enums::{
    HKEY_CURRENT_USER, HKEY_LOCAL_MACHINE, KEY_READ, KEY_WOW64_32KEY, KEY_WOW64_64KEY,
};
use winreg::RegKey;

use super::layout;

/// Our key under `...\Uninstall`. Not `Hoard`: that is the NSIS installer's, and
/// while both exist during a move each has to be removable on its own.
const UNINSTALL_KEY: &str =
    r"Software\Microsoft\Windows\CurrentVersion\Uninstall\services.hoard.saves";
const UNINSTALL_ROOT: &str = r"Software\Microsoft\Windows\CurrentVersion\Uninstall";
const SCHEME_KEY: &str = r"Software\Classes\hoard";
const RUN_KEY: &str = r"Software\Microsoft\Windows\CurrentVersion\Run";
/// The app's value under [`RUN_KEY`]; the service's is `HoardSync`.
const APP_RUN_VALUE: &str = "Hoard";
const APP_EXE: &str = "hoard-desktop.exe";

/// WebView2's Evergreen runtime, as EdgeUpdate registers it.
const WEBVIEW2_CLIENT: &str = "{F3017226-FE2A-4295-8BDF-00C3A9A7E4C5}";
const WEBVIEW2_BOOTSTRAPPER: &str = "https://go.microsoft.com/fwlink/p/?LinkId=2124703";

/// No console window for the helpers we run. Hoard Setup is a windowed program,
/// and a PowerShell flashing up behind it looks like something went wrong.
const CREATE_NO_WINDOW: u32 = 0x0800_0000;

/// Registers the app that [`super::fetch::apply_desktop`] just placed:
/// shortcuts, the link handler and the uninstall entry.
///
/// `desktop_shortcut` only on a first install, or when the old install had
/// one: a shortcut the user deleted does not come back with an update.
pub async fn register(app: &Path, version: &str, desktop_shortcut: bool) -> Result<()> {
    shortcuts(app, desktop_shortcut).await?;
    scheme(app)?;
    uninstall_entry(app, version)?;
    Ok(())
}

/// The counterpart of [`register`]. Best-effort throughout: an uninstall
/// carries on past a shortcut that is already gone.
pub async fn unregister() {
    let hkcu = RegKey::predef(HKEY_CURRENT_USER);
    let _ = hkcu.delete_subkey_all(UNINSTALL_KEY);
    let _ = hkcu.delete_subkey_all(SCHEME_KEY);
    let _ = powershell(
        "foreach ($f in 'Programs','Desktop') { \
           Remove-Item -ErrorAction SilentlyContinue \
             (Join-Path ([Environment]::GetFolderPath($f)) 'Hoard.lnk') }",
    )
    .await;
}

/// The app's own start-at-login line, `HKCU\...\Run\Hoard` as
/// `tauri-plugin-autostart` writes it (`<exe> --silent`, unquoted). Read before
/// an old install comes off.
pub fn app_autostart_line() -> Option<String> {
    RegKey::predef(HKEY_CURRENT_USER)
        .open_subkey_with_flags(RUN_KEY, KEY_READ)
        .ok()?
        .get_value::<String, _>(APP_RUN_VALUE)
        .ok()
        .filter(|line| line.to_ascii_lowercase().contains(APP_EXE))
}

/// Points the app's start at login at `app`, keeping its arguments.
///
/// After a move out of the NSIS folder the line either names the exe that
/// folder no longer has, or is gone (Tauri's uninstaller deletes it unless it
/// runs as an update). Either way the app stopped starting at login until
/// somebody opened it by hand, which is when it registers itself again.
pub fn repoint_app_autostart(before: &str, app: &Path) {
    let Some(line) = repointed(before, app) else {
        return;
    };
    if let Ok((key, _)) = RegKey::predef(HKEY_CURRENT_USER).create_subkey(RUN_KEY) {
        let _ = key.set_value(APP_RUN_VALUE, &line);
    }
}

fn repointed(before: &str, app: &Path) -> Option<String> {
    // Lowercasing ASCII keeps every byte where it was, so the offset holds.
    let at = before.to_ascii_lowercase().find(APP_EXE)?;
    let args = before[at + APP_EXE.len()..].trim_start_matches('"').trim();
    let line = if args.is_empty() {
        app.display().to_string()
    } else {
        format!("{} {args}", app.display())
    };
    (line != before).then_some(line)
}

/// Is there a `Hoard.lnk` on the desktop right now? Asked before an old
/// install is removed, because its uninstaller takes the shortcut with it.
pub async fn has_desktop_shortcut() -> bool {
    powershell(
        "if (Test-Path (Join-Path ([Environment]::GetFolderPath('Desktop')) 'Hoard.lnk')) \
           { exit 0 } else { exit 1 }",
    )
    .await
    .unwrap_or(false)
}

async fn shortcuts(app: &Path, desktop: bool) -> Result<()> {
    let exe = ps_quote(&app.to_string_lossy());
    let dir = ps_quote(&app.parent().unwrap_or(app).to_string_lossy());
    let script = format!(
        "$w = New-Object -ComObject WScript.Shell; \
         function Make($p) {{ $s = $w.CreateShortcut($p); $s.TargetPath = {exe}; \
           $s.WorkingDirectory = {dir}; $s.IconLocation = {exe} + ',0'; \
           $s.Description = 'Hoard'; $s.Save() }}; \
         Make (Join-Path ([Environment]::GetFolderPath('Programs')) 'Hoard.lnk'); \
         if (${desktop}) {{ Make (Join-Path ([Environment]::GetFolderPath('Desktop')) 'Hoard.lnk') }}",
        desktop = if desktop { "true" } else { "false" },
    );
    if !powershell(&script).await? {
        anyhow::bail!("couldn't create the Start menu shortcut");
    }
    Ok(())
}

/// `hoard://`, which the browser hands back to the app at the end of a login
/// when the loopback port is out of reach.
fn scheme(app: &Path) -> Result<()> {
    let hkcu = RegKey::predef(HKEY_CURRENT_USER);
    let (key, _) = hkcu
        .create_subkey(SCHEME_KEY)
        .context("creating the hoard:// handler")?;
    key.set_value("", &"URL:Hoard")?;
    key.set_value("URL Protocol", &"")?;
    let (icon, _) = key.create_subkey("DefaultIcon")?;
    icon.set_value("", &format!("{},0", app.display()))?;
    let (command, _) = key.create_subkey(r"shell\open\command")?;
    command.set_value("", &format!("\"{}\" \"%1\"", app.display()))?;
    Ok(())
}

fn uninstall_entry(app: &Path, version: &str) -> Result<()> {
    let setup = layout::setup_copy()?;
    let root = layout::root_dir()?;
    let hkcu = RegKey::predef(HKEY_CURRENT_USER);
    let (key, _) = hkcu
        .create_subkey(UNINSTALL_KEY)
        .context("creating the uninstall entry")?;
    key.set_value("DisplayName", &"Hoard")?;
    key.set_value("DisplayVersion", &version)?;
    key.set_value("Publisher", &"Hoard")?;
    key.set_value("URLInfoAbout", &"https://hoard.services")?;
    key.set_value("DisplayIcon", &format!("{},0", app.display()))?;
    key.set_value("InstallLocation", &root.to_string_lossy().to_string())?;
    // `--remove` opens Hoard Setup on its uninstall screen: Settings expects a
    // window it can watch. The quiet one is for deployment tools.
    key.set_value(
        "UninstallString",
        &format!("\"{}\" --remove", setup.display()),
    )?;
    key.set_value(
        "QuietUninstallString",
        &format!("\"{}\" --uninstall", setup.display()),
    )?;
    key.set_value("NoModify", &1u32)?;
    key.set_value("NoRepair", &1u32)?;
    let kb = dir_size(&root) / 1024;
    key.set_value("EstimatedSize", &(kb.min(u32::MAX as u64) as u32))?;
    Ok(())
}

/// Keeps a copy of Hoard Setup where the uninstall entry points.
///
/// Renamed into place, so it works while the copy there is the one running
/// (an uninstall started from Settings that turned into a reinstall).
pub fn keep_setup_copy(from: &Path) -> Result<()> {
    let dest = layout::setup_copy()?;
    if same_file(from, &dest) {
        return Ok(());
    }
    if let Some(dir) = dest.parent() {
        std::fs::create_dir_all(dir).with_context(|| format!("creating {}", dir.display()))?;
    }
    let staging = dest.with_extension("new");
    let _ = std::fs::remove_file(&staging);
    std::fs::copy(from, &staging)
        .with_context(|| format!("copying {} to {}", from.display(), staging.display()))?;
    super::fetch::replace_binary(&staging, &dest)
}

fn same_file(a: &Path, b: &Path) -> bool {
    match (std::fs::canonicalize(a), std::fs::canonicalize(b)) {
        (Ok(a), Ok(b)) => a == b,
        _ => false,
    }
}

/// Deletes the uninstaller copy once this process, which may be it, is gone.
pub fn forget_setup_copy() {
    let Ok(path) = layout::setup_copy() else {
        return;
    };
    let _ = std::fs::remove_file(path.with_extension("old"));
    if std::fs::remove_file(&path).is_ok() {
        return;
    }
    // We are that file, and it can't go while we run: a window that finished
    // an uninstall stays open until somebody closes it. PowerShell waits for
    // this process to end and deletes it then. Single quotes only, so the
    // command line survives being quoted for it as one argument.
    let script = format!(
        "Wait-Process -Id {} -ErrorAction SilentlyContinue; \
         Remove-Item -LiteralPath {} -Force -ErrorAction SilentlyContinue",
        std::process::id(),
        ps_quote(&path.to_string_lossy())
    );
    let _ = std::process::Command::new("powershell")
        .args(["-NoProfile", "-NonInteractive", "-Command", &script])
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .creation_flags_no_window()
        .spawn();
}

/// Is the WebView2 runtime there? Without it the app opens to nothing at all.
pub fn has_webview2() -> bool {
    let read = |hive, path: &str, flags| {
        RegKey::predef(hive)
            .open_subkey_with_flags(path, KEY_READ | flags)
            .and_then(|k| k.get_value::<String, _>("pv"))
            .ok()
            .filter(|v| !v.is_empty() && v != "0.0.0.0")
    };
    let client = format!(r"SOFTWARE\Microsoft\EdgeUpdate\Clients\{WEBVIEW2_CLIENT}");
    read(HKEY_LOCAL_MACHINE, &client, KEY_WOW64_32KEY).is_some()
        || read(HKEY_LOCAL_MACHINE, &client, KEY_WOW64_64KEY).is_some()
        || read(HKEY_CURRENT_USER, &client, 0).is_some()
}

/// Installs WebView2 with Microsoft's bootstrapper when it is missing. Windows
/// 11 always has it and most Windows 10 machines got it with Edge; this is for
/// the rest, and it is what the NSIS installer did for them.
pub async fn ensure_webview2(dir: &Path) -> Result<()> {
    if has_webview2() {
        return Ok(());
    }
    tracing::info!("install: WebView2 is missing, running Microsoft's bootstrapper");
    let client = reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(300))
        .build()?;
    let bytes = client
        .get(WEBVIEW2_BOOTSTRAPPER)
        .send()
        .await
        .context("downloading the WebView2 bootstrapper")?
        .error_for_status()?
        .bytes()
        .await?;
    std::fs::create_dir_all(dir)?;
    let exe = dir.join("MicrosoftEdgeWebview2Setup.exe");
    std::fs::write(&exe, &bytes).with_context(|| format!("writing {}", exe.display()))?;
    let status = tokio::process::Command::new(&exe)
        .args(["/silent", "/install"])
        .status()
        .await
        .context("running the WebView2 bootstrapper")?;
    let _ = std::fs::remove_file(&exe);
    if !status.success() || !has_webview2() {
        anyhow::bail!("the WebView2 runtime didn't install ({status})");
    }
    Ok(())
}

/// The product codes of Hoard's per-machine MSI installs, read from the same
/// list Settings shows.
pub fn msi_installs() -> Vec<String> {
    let mut out = Vec::new();
    for view in [KEY_WOW64_64KEY, KEY_WOW64_32KEY] {
        let Ok(root) = RegKey::predef(HKEY_LOCAL_MACHINE)
            .open_subkey_with_flags(UNINSTALL_ROOT, KEY_READ | view)
        else {
            continue;
        };
        for name in root.enum_keys().flatten() {
            if !name.starts_with('{') || out.contains(&name) {
                continue;
            }
            let Ok(entry) = root.open_subkey_with_flags(&name, KEY_READ | view) else {
                continue;
            };
            let display: String = entry.get_value("DisplayName").unwrap_or_default();
            let msi: u32 = entry.get_value("WindowsInstaller").unwrap_or(0);
            if display == "Hoard" && msi == 1 {
                out.push(name);
            }
        }
    }
    out
}

/// Runs a PowerShell one-liner, hidden. `Ok(true)` when it exited 0.
async fn powershell(script: &str) -> Result<bool> {
    let status = tokio::process::Command::new("powershell")
        .args([
            "-NoProfile",
            "-NonInteractive",
            "-ExecutionPolicy",
            "Bypass",
            "-Command",
            script,
        ])
        .creation_flags(CREATE_NO_WINDOW)
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .status()
        .await
        .context("running powershell")?;
    Ok(status.success())
}

/// A PowerShell single-quoted string: the only escape it has is doubling `'`.
fn ps_quote(s: &str) -> String {
    format!("'{}'", s.replace('\'', "''"))
}

fn dir_size(dir: &Path) -> u64 {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return 0;
    };
    entries
        .flatten()
        .map(|e| match e.file_type() {
            Ok(t) if t.is_dir() && e.file_name() != "hoard" => dir_size(&e.path()),
            Ok(t) if t.is_file() => e.metadata().map(|m| m.len()).unwrap_or(0),
            _ => 0,
        })
        .sum()
}

trait NoWindow {
    fn creation_flags_no_window(&mut self) -> &mut Self;
}

impl NoWindow for std::process::Command {
    fn creation_flags_no_window(&mut self) -> &mut Self {
        use std::os::windows::process::CommandExt;
        self.creation_flags(CREATE_NO_WINDOW)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_quote_in_a_path_cannot_end_the_string() {
        assert_eq!(
            ps_quote(r"C:\Users\O'Brien\x.exe"),
            r"'C:\Users\O''Brien\x.exe'"
        );
    }

    #[test]
    fn the_login_line_follows_the_app_and_keeps_its_arguments() {
        let app = Path::new(r"C:\Users\Juan Pérez\AppData\Local\hoard\app\hoard-desktop.exe");
        let old = r"C:\Users\Juan Pérez\AppData\Local\Hoard\hoard-desktop.exe --silent";
        assert_eq!(
            repointed(old, app).as_deref(),
            Some(r"C:\Users\Juan Pérez\AppData\Local\hoard\app\hoard-desktop.exe --silent")
        );
        // Already pointing there: nothing to write.
        assert_eq!(repointed(&repointed(old, app).unwrap(), app), None);
    }
}
