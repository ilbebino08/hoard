//! Getting out of the way of the Hoard that launched us to update itself.
//!
//! Every Hoard up to 1.2.1 updates on Windows by downloading `*-setup.exe` and
//! running it with `/S`, from inside `hoardd`. This program is published under
//! that name so those machines move to the new layout on their next update. But
//! the install it runs stops every `hoardd`, the parent included, and ends the
//! `HoardSync` task: a child that stays in the task's job object would be ended
//! with it, halfway through replacing the binaries.
//!
//! So when the parent is `hoardd.exe`, the first thing we do is start a copy of
//! ourselves outside that job, answer "done" to the old updater, and let the copy
//! wait for it to exit before touching anything.

use windows_sys::Win32::Foundation::{CloseHandle, INVALID_HANDLE_VALUE, WAIT_OBJECT_0};
use windows_sys::Win32::System::Diagnostics::ToolHelp::{
    CreateToolhelp32Snapshot, Process32FirstW, Process32NextW, PROCESSENTRY32W, TH32CS_SNAPPROCESS,
};
use windows_sys::Win32::System::Threading::{
    OpenProcess, WaitForSingleObject, PROCESS_SYNCHRONIZE,
};

const DETACHED_FROM: &str = "--detached-from";

/// `Some(exit code)` when this process should end here because a detached copy
/// carries on; `None` to go ahead with the install in this one.
pub fn step_aside(args: &[String]) -> Option<i32> {
    if let Some(pid) = args
        .iter()
        .position(|a| a == DETACHED_FROM)
        .and_then(|i| args.get(i + 1))
        .and_then(|p| p.parse::<u32>().ok())
    {
        wait_for_exit(pid);
        return None;
    }
    let (parent_pid, parent_name) = parent()?;
    if !parent_name.eq_ignore_ascii_case("hoardd.exe") {
        return None;
    }
    let exe = std::env::current_exe().ok()?;
    let mut forwarded: Vec<String> = args.to_vec();
    forwarded.push(DETACHED_FROM.into());
    forwarded.push(parent_pid.to_string());
    if spawn_outside_job(&exe, &forwarded) {
        Some(0)
    } else {
        // Nothing worked: carry on here and hope the task isn't ended under us.
        None
    }
}

/// Our parent's pid and executable name.
fn parent() -> Option<(u32, String)> {
    let me = std::process::id();
    let entries = processes();
    let ppid = entries.iter().find(|(pid, _, _)| *pid == me)?.1;
    let name = entries.iter().find(|(pid, _, _)| *pid == ppid)?.2.clone();
    Some((ppid, name))
}

/// `(pid, parent pid, exe name)` for every process.
fn processes() -> Vec<(u32, u32, String)> {
    let mut out = Vec::new();
    // SAFETY: plain Win32 calls; the entry is zeroed and sized before use, and
    // the snapshot handle is closed on every path out.
    unsafe {
        let snapshot = CreateToolhelp32Snapshot(TH32CS_SNAPPROCESS, 0);
        if snapshot == INVALID_HANDLE_VALUE {
            return out;
        }
        let mut entry: PROCESSENTRY32W = std::mem::zeroed();
        entry.dwSize = std::mem::size_of::<PROCESSENTRY32W>() as u32;
        if Process32FirstW(snapshot, &mut entry) != 0 {
            loop {
                let len = entry
                    .szExeFile
                    .iter()
                    .position(|c| *c == 0)
                    .unwrap_or(entry.szExeFile.len());
                let name = String::from_utf16_lossy(&entry.szExeFile[..len]);
                out.push((entry.th32ProcessID, entry.th32ParentProcessID, name));
                if Process32NextW(snapshot, &mut entry) == 0 {
                    break;
                }
            }
        }
        CloseHandle(snapshot);
    }
    out
}

fn wait_for_exit(pid: u32) {
    // SAFETY: the handle is checked before use and closed after it.
    unsafe {
        let handle = OpenProcess(PROCESS_SYNCHRONIZE, 0, pid);
        if handle.is_null() {
            return;
        }
        let waited = WaitForSingleObject(handle, 60_000);
        let _ = waited == WAIT_OBJECT_0;
        CloseHandle(handle);
    }
}

/// Starts `exe` outside our job object: by asking to break away from it, and
/// when the job doesn't allow that, through WMI, whose processes are children
/// of the WMI service and in no job of ours.
fn spawn_outside_job(exe: &std::path::Path, args: &[String]) -> bool {
    use std::os::windows::process::CommandExt;
    const DETACHED_PROCESS: u32 = 0x0000_0008;
    const CREATE_NEW_PROCESS_GROUP: u32 = 0x0000_0200;
    const CREATE_BREAKAWAY_FROM_JOB: u32 = 0x0100_0000;
    const CREATE_NO_WINDOW: u32 = 0x0800_0000;

    let spawned = std::process::Command::new(exe)
        .args(args)
        .creation_flags(DETACHED_PROCESS | CREATE_NEW_PROCESS_GROUP | CREATE_BREAKAWAY_FROM_JOB)
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .spawn();
    if spawned.is_ok() {
        return true;
    }

    let quote = |s: &str| format!("\"{}\"", s.replace('"', "\\\""));
    let mut line = quote(&exe.to_string_lossy());
    for arg in args {
        line.push(' ');
        line.push_str(&quote(arg));
    }
    let script = format!(
        "$r = Invoke-CimMethod -ClassName Win32_Process -MethodName Create \
           -Arguments @{{ CommandLine = '{}' }}; exit $r.ReturnValue",
        line.replace('\'', "''")
    );
    std::process::Command::new("powershell")
        .args(["-NoProfile", "-NonInteractive", "-Command", &script])
        .creation_flags(CREATE_NO_WINDOW)
        .status()
        .is_ok_and(|s| s.success())
}
