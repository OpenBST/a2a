// a2a (Agent to Agent) - v0.1.0
// https://github.com/OpenBST/a2a
//
// Author: OpenBST
// License: MIT OR Apache-2.0

//! `a2a doctor` and `a2a status` implementation. SPEC §3.

use anyhow::Result;
use std::env;

pub fn run_doctor() -> Result<()> {
    print_header();
    let bin = super::cursor_agent::locate_binary();
    match &bin {
        Some(p) => {
            println!("[ok] cursor-agent found: {}", p.display());
            let runtime = tokio::runtime::Builder::new_current_thread()
                .enable_all()
                .build()?;
            runtime.block_on(async {
                match super::cursor_agent::run_version_check().await {
                    Ok(v) => println!("[ok] cursor-agent version: {v}"),
                    Err(e) => println!("[!!] could not query version: {e}"),
                }
            });
        }
        None => {
            println!("[!!] cursor-agent NOT in PATH");
        }
    }
    print_profile_summary()?;
    let project_initialised = current_project_initialised();
    println!(
        "project: {}",
        if project_initialised {
            "initialised (.a2a/ marker found)"
        } else {
            "<not initialised in this directory>"
        }
    );
    if bin.is_none() {
        print_install_instructions();
    }
    println!();
    println!("Run `a2a auth add` to register an API key,");
    if !project_initialised {
        println!("then `a2a init` in your project to install Cursor skills + project config,");
    }
    println!("then `a2a ask <topic> --prompt-file <path>` to consult.");
    Ok(())
}

/// True iff the current working directory (or any ancestor) contains
/// a `.a2a/` project marker. Drives the "project initialised?" line
/// in `a2a doctor` and the optional `a2a init` hint at the bottom
/// of doctor output.
///
/// We rely on `find_project_root`'s `is_some()` directly — see its
/// own doc comment for why callers must NOT re-derive
/// `<root>.join(".a2a").is_dir()` (the credentials store at
/// `~/.a2a/` would otherwise be misdetected as a project).
fn current_project_initialised() -> bool {
    let Ok(cwd) = env::current_dir() else {
        return false;
    };
    crate::paths::find_project_root(&cwd).is_some()
}

pub fn print_status() -> Result<()> {
    print_header();
    let bin = super::cursor_agent::locate_binary();
    println!(
        "cursor-agent: {}",
        bin.as_ref()
            .map(|p| p.display().to_string())
            .unwrap_or_else(|| "<NOT FOUND>".into())
    );
    if bin.is_some() {
        let runtime = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()?;
        let status_text = runtime
            .block_on(super::cursor_agent::run_status_check(None))
            .unwrap_or_default();
        let trimmed = status_text.trim();
        if !trimmed.is_empty() {
            println!("cursor login:");
            for line in trimmed.lines() {
                println!("  {line}");
            }
        }
    }
    print_profile_summary()?;
    Ok(())
}

fn print_header() {
    println!(
        "a2a {} ({} {})",
        crate::A2A_VERSION,
        env::consts::OS,
        env::consts::ARCH
    );
}

fn print_profile_summary() -> Result<()> {
    let db = match crate::auth::store::open() {
        Ok(s) => s,
        Err(e) => {
            println!("[!!] could not open credential store: {e:#}");
            return Ok(());
        }
    };
    let profiles = db.list_profiles()?;
    let default = db.get_default_profile()?;
    println!("credentials: {} profile(s)", profiles.len());
    if let Some(d) = default {
        println!("default profile: {d}");
    } else if !profiles.is_empty() {
        println!("default profile: <none set>  (run `a2a auth use <name>`)");
    }
    Ok(())
}

fn print_install_instructions() {
    println!();
    println!("To install Cursor CLI:");
    if cfg!(target_os = "windows") {
        println!("  Windows:");
        println!("    1. Open https://cursor.com/cli in a browser and download the installer");
        println!("    2. After install, restart your terminal so PATH picks up cursor-agent");
        println!("    3. Run: cursor-agent login   (or use API key via `a2a auth add`)");
    } else if cfg!(target_os = "macos") {
        println!("  macOS:");
        println!("    curl -fsSL https://cursor.com/install | sh");
    } else {
        println!("  Linux:");
        println!("    curl -fsSL https://cursor.com/install | sh");
    }
    println!();
    println!("API key (recommended for headless / CI use):");
    println!("  1. Open https://cursor.com/dashboard → Integrations → Add API key");
    println!("  2. Copy the key (format: key_xxxxxxxxxxxxxxxx...)");
    println!("  3. a2a auth add  (paste when prompted; profile name auto-derived from key tail)");
}
