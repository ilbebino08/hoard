//! `hoard install`: leaves this machine with all the Hoard it should have, at one
//! single version, and nothing else.
//!
//! The terminal installer downloads the core, checks it and runs this from the
//! folder it unpacked; from there it is the same sequence Hoard Setup runs
//! (`hoardd::setup`): the core goes where it lives, older copies of Hoard come
//! off, the service starts from the new core, and on a desktop the app comes
//! too.
//!
//! It is also what somebody runs by hand when something was left half done (no
//! network during the install, a cancelled password prompt), and what the
//! service runs on its own to move a machine off an older layout.

use std::io::IsTerminal;
use std::path::PathBuf;

use anyhow::{Context, Result};

pub use hoardd::setup::Want;
use hoardd::setup::{Plan, Source};

pub async fn run(want: Want, version: Option<String>, from: Option<PathBuf>) -> Result<()> {
    let own = hoard_agent::update::current();
    let exe = std::env::current_exe().context("resolving our own path")?;
    let here = match &from {
        Some(dir) => dir.clone(),
        None => exe
            .parent()
            .context("our own path has no parent")?
            .to_path_buf(),
    };
    // A folder named on purpose brings its app and its Hoard Setup too.
    let (app_file, setup_binary) = match &from {
        Some(dir) => hoardd::setup::from_dir(dir),
        None => (None, None),
    };
    // Another version than this binary is a download; this one is already here.
    let pinned = version
        .as_deref()
        .map(|v| v.trim_start_matches('v').to_string())
        .filter(|v| v != own);
    let source = match &pinned {
        Some(_) => Source::Download,
        None => Source::Local(here),
    };
    // Inside `curl … | sh` stdin is the script: nobody can answer a prompt.
    let interactive =
        std::env::var_os("HOARD_NONINTERACTIVE").is_none() && std::io::stdin().is_terminal();

    println!(
        "hoard {} — setting up this machine…",
        pinned.as_deref().unwrap_or(own)
    );
    let outcome = hoardd::setup::run(Plan {
        version: pinned,
        want,
        source,
        interactive,
        setup_binary,
        app_file,
    })
    .await?;

    let core = hoard_agent::install::layout::core_dir()?;
    println!("  core:    {} (service running)", core.display());
    match &outcome.app {
        Some(app) => println!("  desktop: {}", app.display()),
        None => println!("  desktop: not installed"),
    }
    // The install closed the app to replace it. When the service runs this on
    // its own there is nobody to notice it gone.
    if let (true, Some(app)) = (outcome.app_was_running, &outcome.app) {
        hoardd::setup::reopen_app(app);
    }
    for warning in &outcome.warnings {
        eprintln!("  warning: {warning}");
    }
    if !outcome.left.is_empty() {
        eprintln!("\nAn older Hoard is still here and needs an administrator to remove:");
        for item in &outcome.left {
            eprintln!("  - {item}");
        }
        eprintln!("Run `hoard install` again from a terminal, or open Hoard Setup, to finish.");
    }
    Ok(())
}
